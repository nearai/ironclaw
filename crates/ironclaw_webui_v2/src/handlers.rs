//! WebChat v2 HTTP handlers.
//!
//! Every handler:
//!
//! 1. Receives an authenticated caller as an `Extension<WebUiAuthenticatedCaller>`.
//!    Host composition is responsible for running the bearer-token middleware
//!    that builds that extension; the handler never sees a raw bearer token.
//! 2. Dispatches through [`RebornServicesApi`]. No direct access to the
//!    dispatcher, `HostRuntime`, run-state, DB stores, or any runtime lane.
//! 3. Maps every error through [`WebUiV2HttpError`] so the wire shape stays
//!    redacted and stable.
//!
//! [`RebornServicesApi`]: ironclaw_product_workflow::RebornServicesApi

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::SinkExt;
use futures::stream::Stream;
use ironclaw_product_workflow::{
    CodexLoginStart, FsMount, LifecyclePackageKind, LifecyclePackageRef, LlmConfigSnapshot,
    LlmModelsResult, LlmProbeRequest, LlmProbeResult, NearAiLoginRequest, NearAiLoginStart,
    NearAiWalletLoginRequest, NearAiWalletLoginResult, ProductWorkflowError, ProjectFsFile,
    ProjectionCursor, RebornAddMemberRequest, RebornAttachmentRequest,
    RebornAutomationMutationResponse, RebornCancelRunResponse,
    RebornConnectableChannelListResponse, RebornCreateProjectRequest, RebornCreateThreadResponse,
    RebornDeleteProjectRequest, RebornDeleteThreadRequest, RebornDeleteThreadResponse,
    RebornExtensionActionResponse, RebornExtensionListResponse, RebornExtensionRegistryResponse,
    RebornFsListRequest, RebornFsListResponse, RebornFsMountsResponse, RebornFsReadRequest,
    RebornFsStatRequest, RebornFsStatResponse, RebornGetProjectRequest,
    RebornListAutomationsResponse, RebornListMembersRequest, RebornListMembersResponse,
    RebornListProjectsRequest, RebornListProjectsResponse, RebornListThreadsResponse,
    RebornLogQueryRequest, RebornLogQueryResponse, RebornOperatorCommandPlaneResponse,
    RebornOperatorConfigGetResponse, RebornOperatorConfigListResponse,
    RebornOperatorConfigSetRequest, RebornOperatorConfigValidateRequest,
    RebornOperatorConfigValidateResponse, RebornOperatorLogsQuery,
    RebornOperatorServiceLifecycleRequest, RebornOperatorSetupRequest, RebornOperatorSetupResponse,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundPreferencesResponse,
    RebornProjectFsListRequest, RebornProjectFsListResponse, RebornProjectFsReadRequest,
    RebornProjectFsStatRequest, RebornProjectFsStatResponse, RebornProjectMemberInfo,
    RebornProjectResponse, RebornRemoveMemberRequest, RebornResolveGateResponse, RebornServicesApi,
    RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
    RebornSetOutboundPreferencesRequest, RebornSetupExtensionResponse, RebornSkillActionResponse,
    RebornSkillContentResponse, RebornSkillListResponse, RebornSkillSearchResponse,
    RebornStreamEventsRequest, RebornSubmitTurnResponse, RebornTimelineRequest,
    RebornTimelineResponse, RebornTraceCreditsResponse, RebornTraceHoldAuthorizeResponse,
    RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest, SetActiveLlmRequest,
    SettingsToolPermissionState, UpsertLlmProviderRequest, WebUiAttachmentCapabilities,
    WebUiAuthenticatedCaller, WebUiCancelRunRequest, WebUiCreateThreadRequest,
    WebUiInboundValidationCode, WebUiInboundValidationError, WebUiListAutomationsRequest,
    WebUiListThreadsRequest, WebUiResolveGateRequest, WebUiSendMessageRequest,
    WebUiSetupExtensionRequest, webui_attachment_capabilities,
};
use serde::{Deserialize, Serialize};

use crate::error::WebUiV2HttpError;
use crate::router::{WebUiV2Capabilities, WebUiV2State};
use crate::schema::WebChatV2EventFrame;
use crate::sse_capacity::{SSE_MAX_LIFETIME, SseSlot};

// Session bootstrap must stay cheap and non-blocking: this flag only tunes
// initial approval UI state. It is mutable through `/settings/tools`, so do
// not cache it across requests; the settings route remains authoritative.
const GLOBAL_AUTO_APPROVE_FEATURE_TIMEOUT: Duration = Duration::from_millis(250);
const SETTINGS_TOOLS_AUTO_APPROVE_KEY: &str = "agent.auto_approve_tools";
const SETTINGS_TOOL_CONFIG_PREFIX: &str = "tool.";
const SETTINGS_TOOL_CAPABILITY_ID_MAX_BYTES: usize =
    OPERATOR_CONFIG_KEY_MAX_BYTES - SETTINGS_TOOL_CONFIG_PREFIX.len();

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
        state.services().global_auto_approve_enabled(caller),
    )
    .await
    {
        Ok(Ok(enabled)) => enabled,
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
    let response = state.services().create_thread(caller, body).await?;
    Ok(Json(response))
}

/// `DELETE /api/webchat/v2/threads/{thread_id}`
pub async fn delete_thread(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
) -> Result<Json<RebornDeleteThreadResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .delete_thread(caller, RebornDeleteThreadRequest { thread_id })
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
    let response = state.services().submit_turn(caller, body).await?;
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
    let response = state.services().get_timeline(caller, request).await?;
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
    let response = state.services().list_project_dir(caller, request).await?;
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
    let response = state.services().stat_project_path(caller, request).await?;
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
    let file = state.services().read_project_file(caller, request).await?;
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
/// mount-relative path (absent/blank means the mount root for listing).
#[derive(Debug, Deserialize)]
pub struct FsBrowseQuery {
    pub mount: FsMount,
    #[serde(default)]
    pub path: Option<String>,
}

/// `GET /api/webchat/v2/fs/mounts`
///
/// List the mounts the read-only filesystem viewer can browse for this caller.
pub async fn list_fs_mounts(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornFsMountsResponse>, WebUiV2HttpError> {
    let response = state.services().list_fs_mounts(caller).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/fs/list?mount=…&path=…`
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
    };
    let response = state.services().browse_fs_dir(caller, request).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/fs/stat?mount=…&path=…`
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
    };
    let response = state.services().stat_fs_path(caller, request).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/fs/content?mount=…&path=…`
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
    };
    let file = state.services().read_fs_file(caller, request).await?;
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
    let response = state.services().list_projects(caller, request).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/projects`
pub async fn create_project(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<RebornCreateProjectRequest>,
) -> Result<Json<RebornProjectResponse>, WebUiV2HttpError> {
    let response = state.services().create_project(caller, body).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/projects/{project_id}`
pub async fn get_project(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(project_id): Path<String>,
) -> Result<Json<RebornProjectResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .get_project(caller, RebornGetProjectRequest { project_id })
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
    let response = state.services().update_project(caller, body).await?;
    Ok(Json(response))
}

/// `DELETE /api/webchat/v2/projects/{project_id}`
pub async fn delete_project(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(project_id): Path<String>,
) -> Result<StatusCode, WebUiV2HttpError> {
    state
        .services()
        .delete_project(caller, RebornDeleteProjectRequest { project_id })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/webchat/v2/projects/{project_id}/members`
pub async fn list_project_members(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(project_id): Path<String>,
) -> Result<Json<RebornListMembersResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .list_project_members(caller, RebornListMembersRequest { project_id })
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
    let response = state.services().add_project_member(caller, body).await?;
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
    let response = state
        .services()
        .update_project_member_role(caller, body)
        .await?;
    Ok(Json(response))
}

/// `DELETE /api/webchat/v2/projects/{project_id}/members/{user_id}`
pub async fn remove_project_member(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path((project_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, WebUiV2HttpError> {
    state
        .services()
        .remove_project_member(
            caller,
            RebornRemoveMemberRequest {
                project_id,
                user_id,
            },
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
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
    let attachment = state
        .services()
        .read_attachment(
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
/// Until the facade gains a true subscription API, the handler drains and
/// polls in a loop. Drain-only semantics are documented on
/// [`RebornServicesApi::stream_events`].
///
/// [`WebChatV2EventFrame`]: crate::schema::WebChatV2EventFrame
/// [`RebornServicesApi::stream_events`]: ironclaw_product_workflow::RebornServicesApi::stream_events
/// [`SSE_MAX_LIFETIME`]: crate::sse_capacity::SSE_MAX_LIFETIME
pub async fn stream_events(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<StreamEventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, WebUiV2HttpError> {
    let slot = state
        .sse_capacity()
        .try_acquire(&caller.tenant_id, &caller.user_id)
        .ok_or_else(sse_concurrency_exhausted)?;
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
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(SSE_KEEPALIVE_INTERVAL)))
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

fn build_sse_stream(
    services: std::sync::Arc<dyn RebornServicesApi>,
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
        let _slot_guard = slot;
        let started_at = tokio::time::Instant::now();
        let mut after_cursor = initial_cursor.and_then(parse_cursor_token);
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
            match tokio::time::timeout(
                remaining,
                services.stream_events(caller.clone(), request),
            )
            .await
            {
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
                        let frame = WebChatV2EventFrame::from_outbound(envelope);
                        let id = cursor_token(frame.cursor());
                        match serde_json::to_string(&frame) {
                            Ok(payload) => {
                                let mut event = Event::default().event(frame.event_name()).data(payload);
                                if let Some(id) = id {
                                    event = event.id(id);
                                }
                                yield Ok(event);
                            }
                            Err(error) => {
                                // debug, not warn: this is an internal
                                // diagnostic, not user-facing status, and
                                // info!/warn! corrupts the REPL/TUI per
                                // CLAUDE.md.
                                tracing::debug!(
                                    target = "ironclaw_webui_v2::sse",
                                    error = %error,
                                    "failed to serialize WebChatV2EventFrame for SSE",
                                );
                            }
                        }
                    }
                    idle_polls = if had_events {
                        0
                    } else {
                        idle_polls.saturating_add(1)
                    };
                    // Bound the poll sleep too so we never oversleep past the
                    // lifetime budget; the top-of-loop check then fires.
                    let sleep_for = sse_poll_interval_for_idle_polls(idle_polls)
                        .min(SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed()));
                    if sleep_for.is_zero() {
                        return;
                    }
                    tokio::time::sleep(sleep_for).await;
                }
                Ok(Err(error)) => {
                    // Surface a redacted error event and close the stream.
                    // Reconnect logic is the browser's responsibility.
                    tracing::debug!(
                        target = "ironclaw_webui_v2::sse",
                        error = ?error,
                        "facade rejected SSE drain; closing stream",
                    );
                    let payload = SseErrorPayload {
                        error: error.code,
                        kind: error.kind,
                        retryable: error.retryable,
                    };
                    yield Ok(Event::default()
                        .event("error")
                        .json_data(payload)
                        .expect("SseErrorPayload is a tagged enum + bool with derived Serialize; cannot fail")); // safety: typed struct with derived Serialize on serde-compatible fields only
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
    let response = state.services().cancel_run(caller, body).await?;
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
    let response = state.services().resolve_gate(caller, body).await?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct ResolveGatePath {
    pub thread_id: String,
    pub run_id: String,
    pub gate_ref: String,
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
    let request = WebUiListThreadsRequest {
        limit: query.limit,
        cursor: query.cursor,
        candidate_thread_id: query.candidate_thread_id,
        needs_approval: query.needs_approval,
    };
    let response = state.services().list_threads(caller, request).await?;
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
    let response = state.services().list_automations(caller, request).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/automations/:automation_id/pause`
pub async fn pause_automation(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(automation_id): Path<String>,
) -> Result<Json<RebornAutomationMutationResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .pause_automation(caller, automation_id)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/automations/:automation_id/resume`
pub async fn resume_automation(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(automation_id): Path<String>,
) -> Result<Json<RebornAutomationMutationResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .resume_automation(caller, automation_id)
        .await?;
    Ok(Json(response))
}

/// `DELETE /api/webchat/v2/automations/:automation_id`
pub async fn delete_automation(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(automation_id): Path<String>,
) -> Result<Json<RebornAutomationMutationResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .delete_automation(caller, automation_id)
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
    let response = state.services().trace_credits(caller).await?;
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
    let response = state
        .services()
        .authorize_trace_hold(caller, submission_id)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/channels/connectable`
pub async fn list_connectable_channels(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornConnectableChannelListResponse>, WebUiV2HttpError> {
    let response = state.services().list_connectable_channels(caller).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/outbound/preferences`
pub async fn get_outbound_preferences(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornOutboundPreferencesResponse>, WebUiV2HttpError> {
    let response = state.services().get_outbound_preferences(caller).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/outbound/preferences`
///
/// Body shape: [`RebornSetOutboundPreferencesRequest`]. Sending
/// `{"final_reply_target_id": null}` clears the configured final-reply target.
pub async fn set_outbound_preferences(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<RebornSetOutboundPreferencesRequest>,
) -> Result<Json<RebornOutboundPreferencesResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .set_outbound_preferences(caller, body)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/outbound/targets`
pub async fn list_outbound_delivery_targets(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornOutboundDeliveryTargetListResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .list_outbound_delivery_targets(caller)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/extensions`
pub async fn list_extensions(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornExtensionListResponse>, WebUiV2HttpError> {
    let response = state.services().list_extensions(caller).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/skills`
pub async fn list_skills(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornSkillListResponse>, WebUiV2HttpError> {
    let response = state.services().list_skills(caller).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/skills/search`
pub async fn search_skills(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<SearchSkillsBody>,
) -> Result<Json<RebornSkillSearchResponse>, WebUiV2HttpError> {
    let response = state.services().search_skills(caller, body.query).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/skills/install`
pub async fn install_skill(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<InstallSkillBody>,
) -> Result<Json<RebornSkillActionResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .install_skill(caller, body.name, body.content)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/skills/{name}`
pub async fn get_skill_content(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(SkillPath { name }): Path<SkillPath>,
) -> Result<Json<RebornSkillContentResponse>, WebUiV2HttpError> {
    let response = state.services().read_skill_content(caller, name).await?;
    Ok(Json(response))
}

/// `PUT /api/webchat/v2/skills/{name}`
pub async fn update_skill(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(SkillPath { name }): Path<SkillPath>,
    Json(body): Json<UpdateSkillBody>,
) -> Result<Json<RebornSkillActionResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .update_skill(caller, name, body.content)
        .await?;
    Ok(Json(response))
}

/// `DELETE /api/webchat/v2/skills/{name}`
pub async fn remove_skill(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(SkillPath { name }): Path<SkillPath>,
) -> Result<Json<RebornSkillActionResponse>, WebUiV2HttpError> {
    let response = state.services().remove_skill(caller, name).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/skills/{name}/auto-activate`
pub async fn set_skill_auto_activate(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(SkillPath { name }): Path<SkillPath>,
    Json(body): Json<SetSkillAutoActivateBody>,
) -> Result<Json<RebornSkillActionResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .set_skill_auto_activate(caller, name, body.enabled)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/skills/auto-activate-learned`
pub async fn set_auto_activate_learned(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<SetSkillAutoActivateBody>,
) -> Result<Json<RebornSkillActionResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .set_auto_activate_learned(caller, body.enabled)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/extensions/registry`
pub async fn list_extension_registry(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornExtensionRegistryResponse>, WebUiV2HttpError> {
    let response = state.services().list_extension_registry(caller).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/extensions/install`
pub async fn install_extension(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<InstallExtensionBody>,
) -> Result<Json<RebornExtensionActionResponse>, WebUiV2HttpError> {
    let package_ref = extension_package_ref_for_request(Ok(body.package_ref), "package_ref")?;
    let response = state
        .services()
        .install_extension(caller, package_ref)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/extensions/{package_id}/activate`
pub async fn activate_extension(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(ExtensionPackagePath { package_id }): Path<ExtensionPackagePath>,
) -> Result<Json<RebornExtensionActionResponse>, WebUiV2HttpError> {
    let package_ref = extension_package_ref_for_request(
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id),
        "package_id",
    )?;
    let response = state
        .services()
        .activate_extension(caller, package_ref)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/extensions/{package_id}/remove`
pub async fn remove_extension(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(ExtensionPackagePath { package_id }): Path<ExtensionPackagePath>,
) -> Result<Json<RebornExtensionActionResponse>, WebUiV2HttpError> {
    let package_ref = extension_package_ref_for_request(
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id),
        "package_id",
    )?;
    let response = state
        .services()
        .remove_extension(caller, package_ref)
        .await?;
    Ok(Json(response))
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
    let response = state
        .services()
        .setup_extension(caller, package_ref, WebUiSetupExtensionRequest::default())
        .await?;
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
    let response = state
        .services()
        .setup_extension(caller, package_ref, body)
        .await?;
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

/// `GET /api/webchat/v2/operator/setup`
pub async fn get_operator_setup(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<RebornOperatorSetupResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = state.services().get_operator_setup(caller).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/operator/setup`
pub async fn run_operator_setup(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Json(body): Json<RebornOperatorSetupRequest>,
) -> Result<Json<RebornOperatorSetupResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = state.services().run_operator_setup(caller, body).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/settings/tools`
pub async fn list_settings_tools(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(_capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<RebornOperatorConfigListResponse>, WebUiV2HttpError> {
    let mut response = state.services().list_operator_config(caller).await?;
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
    let response = state
        .services()
        .set_operator_config_key(
            caller,
            SETTINGS_TOOLS_AUTO_APPROVE_KEY.to_string(),
            RebornOperatorConfigSetRequest {
                value: serde_json::json!(body.enabled),
            },
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
    let response = state
        .services()
        .set_operator_config_key(
            caller,
            key,
            RebornOperatorConfigSetRequest {
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
    let response = state.services().list_operator_config(caller).await?;
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

/// `GET /api/webchat/v2/operator/config/{key}`
pub async fn get_operator_config_key(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Path(OperatorConfigKeyPath { key }): Path<OperatorConfigKeyPath>,
) -> Result<Json<RebornOperatorConfigGetResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let key = validate_operator_config_key(key)?;
    let response = state
        .services()
        .get_operator_config_key(caller, key)
        .await?;
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
    let response = state
        .services()
        .set_operator_config_key(caller, key, body)
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
    let response = state
        .services()
        .validate_operator_config(caller, body)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/operator/diagnostics`
pub async fn get_operator_diagnostics(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = state.services().get_operator_diagnostics(caller).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/operator/status`
pub async fn get_operator_status(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = state.services().get_operator_status(caller).await?;
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
    Query(query): Query<RebornOperatorLogsQuery>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = state.services().query_operator_logs(caller, query).await?;
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
    let request = RebornLogQueryRequest {
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
    let response = state.services().query_logs(caller, request).await?;
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
    let response = state
        .services()
        .run_operator_service_lifecycle(caller, body)
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
    let response = state.services().get_llm_config(caller).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/providers`
pub async fn upsert_llm_provider(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Json(body): Json<UpsertLlmProviderRequest>,
) -> Result<Json<LlmConfigSnapshot>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = state.services().upsert_llm_provider(caller, body).await?;
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
    let response = state
        .services()
        .delete_llm_provider(caller, provider_id)
        .await?;
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
    let response = state.services().set_active_llm(caller, body).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/test-connection`
pub async fn test_llm_connection(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Json(body): Json<LlmProbeRequest>,
) -> Result<Json<LlmProbeResult>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = state.services().test_llm_connection(caller, body).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/list-models`
pub async fn list_llm_models(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Json(body): Json<LlmProbeRequest>,
) -> Result<Json<LlmModelsResult>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = state.services().list_llm_models(caller, body).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/nearai/login`
pub async fn start_nearai_login(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    headers: HeaderMap,
    Json(mut body): Json<NearAiLoginRequest>,
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
        body.origin = origin.to_string();
    }
    let response = state.services().start_nearai_login(caller, body).await?;
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
    Json(body): Json<NearAiWalletLoginRequest>,
) -> Result<Json<NearAiWalletLoginResult>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = state
        .services()
        .complete_nearai_wallet_login(caller, body)
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
    let response = state.services().start_codex_login(caller).await?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct ExtensionPackagePath {
    pub package_id: String,
}

#[derive(Debug, Deserialize)]
pub struct InstallExtensionBody {
    pub package_ref: LifecyclePackageRef,
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
/// accepts the WS upgrade, drains the same `RebornServicesApi::
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
        .try_acquire(&caller.tenant_id, &caller.user_id)
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
    services: std::sync::Arc<dyn RebornServicesApi>,
    caller: WebUiAuthenticatedCaller,
    thread_id: String,
    initial_cursor: Option<String>,
    slot: SseSlot,
    mut socket: axum::extract::ws::WebSocket,
) {
    // Mirror the SSE generator: own the slot guard, bound stream
    // lifetime, drain stream_events on the same poll cadence, emit
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
    let _slot_guard = slot;
    let started_at = tokio::time::Instant::now();
    let mut after_cursor = initial_cursor.and_then(parse_cursor_token);
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
                idle_polls = if had_events {
                    0
                } else {
                    idle_polls.saturating_add(1)
                };
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
}
