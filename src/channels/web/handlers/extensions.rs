//! Extension management API handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;

pub async fn extensions_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<ExtensionListResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let installed = ext_mgr
        .list(None, false, &user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut owner_bound_channels = std::collections::HashSet::new();
    for ext in &installed {
        if ext.kind == crate::extensions::ExtensionKind::WasmChannel
            && ext_mgr.has_wasm_channel_owner_binding(&ext.name).await
        {
            owner_bound_channels.insert(ext.name.clone());
        }
    }
    let extensions = installed
        .into_iter()
        .map(|ext| {
            let owner_bound = owner_bound_channels.contains(&ext.name);
            crate::channels::web::types::extension_info_from_installed(ext, owner_bound)
        })
        .collect();

    Ok(Json(ExtensionListResponse { extensions }))
}

pub async fn extensions_tools_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Result<Json<ToolListResponse>, (StatusCode, String)> {
    let registry = state.tool_registry.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Tool registry not available".to_string(),
    ))?;

    let definitions = registry.tool_definitions().await;
    let tools = definitions
        .into_iter()
        .map(|td| ToolInfo {
            name: td.name,
            description: td.description,
        })
        .collect();

    Ok(Json(ToolListResponse { tools }))
}

pub async fn extensions_install_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<InstallExtensionRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let kind_hint = req.kind.as_deref().and_then(|k| match k {
        "mcp_server" => Some(crate::extensions::ExtensionKind::McpServer),
        "wasm_tool" => Some(crate::extensions::ExtensionKind::WasmTool),
        "wasm_channel" => Some(crate::extensions::ExtensionKind::WasmChannel),
        "channel_relay" => Some(crate::extensions::ExtensionKind::ChannelRelay),
        "acp_agent" => Some(crate::extensions::ExtensionKind::AcpAgent),
        _ => None,
    });

    match ext_mgr
        .install(&req.name, req.url.as_deref(), kind_hint, &user.user_id)
        .await
    {
        Ok(result) => Ok(Json(ActionResponse::ok(result.message))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

pub async fn extensions_remove_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.remove(&name, &user.user_id).await {
        Ok(message) => Ok(Json(ActionResponse::ok(message))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

#[cfg(test)]
mod tests {
    use crate::channels::web::types::{ExtensionActivationStatus, extension_activation_status};
    use crate::extensions::{ExtensionKind, InstalledExtension};

    fn active_authenticated_wasm_channel(name: &str) -> InstalledExtension {
        InstalledExtension {
            name: name.to_string(),
            kind: ExtensionKind::WasmChannel,
            display_name: None,
            description: None,
            url: None,
            authenticated: true,
            active: true,
            tools: Vec::new(),
            needs_setup: false,
            has_auth: false,
            installed: true,
            activation_error: None,
            version: None,
        }
    }

    #[test]
    fn active_authenticated_wasm_channel_without_owner_binding_is_active() {
        let ext = active_authenticated_wasm_channel("discord");
        assert_eq!(
            extension_activation_status(&ext),
            Some(ExtensionActivationStatus::Active)
        );
    }
}
