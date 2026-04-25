//! MCP prompts HTTP API.
//!
//! - `GET  /api/prompts` — list MCP prompts across the caller's active
//!   servers. JSON-shaped version of the `/prompts` slash command output.
//! - `POST /api/prompts/get` — fetch a specific prompt's rendered messages
//!   for a preview UI. Body: `{ server, name, arguments }`.
//!
//! Both handlers delegate multi-tenancy scoping to
//! [`crate::extensions::ExtensionManager::list_prompts_for_user`] /
//! [`crate::extensions::ExtensionManager::get_prompt_for_user`], which are
//! the single source of truth shared with the `/prompts` slash command and
//! the dispatcher-level `/server:prompt-name` mention expander.

use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::platform::state::GatewayState;
use crate::extensions::{ExtensionError, ServerPromptsEntry};
use crate::tools::mcp::GetPromptResult;

#[derive(Debug, Serialize)]
pub struct PromptsListResponse {
    pub servers: Vec<ServerPromptsEntry>,
}

pub(crate) async fn prompts_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<PromptsListResponse>, (StatusCode, String)> {
    let ext_mgr_opt = state.extension_manager.as_ref(); // dispatch-exempt: read-only MCP prompts aggregation
    let ext_mgr = ext_mgr_opt.ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "extensions not enabled".to_string(),
    ))?;
    let servers = ext_mgr
        .list_prompts_for_user(&user.user_id)
        .await
        .map_err(|e| {
            tracing::warn!(user_id = %user.user_id, error = %e, "list_prompts_for_user failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to list MCP prompts".to_string(),
            )
        })?;
    Ok(Json(PromptsListResponse { servers }))
}

#[derive(Debug, Deserialize)]
pub struct PromptsGetRequest {
    pub server: String,
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct PromptsGetResponse {
    pub server: String,
    pub name: String,
    pub result: GetPromptResult,
}

pub(crate) async fn prompts_get_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<PromptsGetRequest>,
) -> Result<Json<PromptsGetResponse>, (StatusCode, String)> {
    let ext_mgr_opt = state.extension_manager.as_ref(); // dispatch-exempt: read-only MCP prompts/get fetch
    let ext_mgr = ext_mgr_opt.ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "extensions not enabled".to_string(),
    ))?;
    let result = ext_mgr
        .get_prompt_for_user(
            &user.user_id,
            &body.server,
            &body.name,
            serde_json::Value::Object(body.arguments),
        )
        .await
        .map_err(|e| get_prompt_error_to_status(&user.user_id, &body.server, &body.name, e))?;
    Ok(Json(PromptsGetResponse {
        server: body.server,
        name: body.name,
        result,
    }))
}

/// Map an `ExtensionError` from `get_prompt_for_user` to an HTTP status +
/// user-safe message.
///
/// - `NotInstalled` / `NotActive` → 404 (caller referenced a server they
///   don't have active)
/// - `PromptNotFound` → 404 (server is active but doesn't advertise the
///   requested prompt name)
/// - `MissingRequiredArgs` → 400; `Display` renders the prompt name and
///   the missing args so the client can surface a useful message
/// - Everything else → 500 with a generic message; the full error is logged
///   at `warn` for operator visibility. Matches `.claude/rules/error-handling.md`
///   "Error Boundaries at the Channel Edge" — no transport/config internals
///   cross the user boundary.
fn get_prompt_error_to_status(
    user_id: &str,
    server: &str,
    name: &str,
    err: ExtensionError,
) -> (StatusCode, String) {
    match err {
        ExtensionError::NotInstalled(msg) | ExtensionError::NotActive(msg) => {
            (StatusCode::NOT_FOUND, msg)
        }
        err @ ExtensionError::PromptNotFound { .. } => (StatusCode::NOT_FOUND, err.to_string()),
        err @ ExtensionError::MissingRequiredArgs { .. } => {
            (StatusCode::BAD_REQUEST, err.to_string())
        }
        other => {
            tracing::warn!(
                user_id = %user_id,
                server = %server,
                prompt = %name,
                error = %other,
                "get_prompt_for_user failed",
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to fetch MCP prompt".to_string(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_active_maps_to_404() {
        let (status, body) = get_prompt_error_to_status(
            "u",
            "notion",
            "create-page",
            ExtensionError::NotActive("MCP server 'notion' is not active for this user".into()),
        );
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("notion"));
    }

    #[test]
    fn not_installed_maps_to_404() {
        let (status, _) = get_prompt_error_to_status(
            "u",
            "notion",
            "create-page",
            ExtensionError::NotInstalled("not installed".into()),
        );
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn prompt_not_found_maps_to_404() {
        // Distinct 404 from `NotActive` — the server is active, but the
        // specific prompt name doesn't exist. The HTTP boundary preserves
        // the distinction so clients can tell "fix your server selection"
        // apart from "fix your prompt name".
        let err = ExtensionError::PromptNotFound {
            server: "notion".into(),
            prompt: "create-page".into(),
        };
        let (status, body) = get_prompt_error_to_status("u", "notion", "create-page", err);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(
            body.contains("create-page") && body.contains("notion"),
            "404 body should name both server and prompt, got: {body}"
        );
    }

    #[test]
    fn missing_required_arg_maps_to_400() {
        // Typed variant — the HTTP boundary matches on the enum shape,
        // not a substring of the message. Immune to upstream message
        // rephrases.
        let err = ExtensionError::MissingRequiredArgs {
            prompt: "create-page".into(),
            missing: vec!["parent_id".into()],
        };
        let (status, body) = get_prompt_error_to_status("u", "notion", "create-page", err);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body.contains("parent_id"),
            "400 body should name the missing arg, got: {body}"
        );
        assert!(
            body.contains("create-page"),
            "400 body should name the prompt, got: {body}"
        );
    }

    #[test]
    fn missing_required_arg_lists_all_missing() {
        let err = ExtensionError::MissingRequiredArgs {
            prompt: "create-page".into(),
            missing: vec!["parent_id".into(), "title".into()],
        };
        let (status, body) = get_prompt_error_to_status("u", "notion", "create-page", err);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("parent_id") && body.contains("title"));
    }

    #[test]
    fn generic_activation_failed_does_not_leak_internals() {
        // Transport / config errors must NOT reach the user verbatim per
        // `.claude/rules/error-handling.md` "Error Boundaries at the Channel Edge".
        let err = ExtensionError::ActivationFailed(
            "transport: dial tcp 10.0.0.1:5432 i/o timeout (attempt 3/3)".into(),
        );
        let (status, body) = get_prompt_error_to_status("u", "notion", "create-page", err);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            !body.contains("10.0.0.1") && !body.contains("tcp"),
            "generic 500 must NOT include transport internals, got: {body}"
        );
    }

    #[test]
    fn config_error_does_not_leak_internals() {
        let err = ExtensionError::Config(
            "Failed to read ~/.ironclaw/mcp-servers.json: Permission denied (os error 13)".into(),
        );
        let (status, body) = get_prompt_error_to_status("u", "notion", "create-page", err);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            !body.contains("/.ironclaw/") && !body.contains("os error"),
            "generic 500 must NOT include filesystem internals, got: {body}"
        );
    }
}
