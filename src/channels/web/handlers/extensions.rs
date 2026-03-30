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

pub(crate) fn derive_activation_status(
    ext: &crate::extensions::InstalledExtension,
    pairing_store: &crate::pairing::PairingStore,
    has_owner_binding: bool,
) -> Option<ExtensionActivationStatus> {
    if ext.kind == crate::extensions::ExtensionKind::WasmChannel {
        let allowlist_exists = pairing_store
            .has_allow_from_file(&ext.name)
            .unwrap_or(false);
        let has_paired = pairing_store
            .read_allow_from(&ext.name)
            .map(|list| !list.is_empty())
            .unwrap_or(false);
        classify_wasm_channel_activation(
            ext,
            has_paired,
            has_owner_binding || (ext.active && !allowlist_exists),
        )
    } else if ext.kind == crate::extensions::ExtensionKind::ChannelRelay {
        Some(if ext.active {
            ExtensionActivationStatus::Active
        } else if ext.authenticated {
            ExtensionActivationStatus::Configured
        } else {
            ExtensionActivationStatus::Installed
        })
    } else {
        None
    }
}

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

    let pairing_store = crate::pairing::PairingStore::new();
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
            let activation_status = derive_activation_status(
                &ext,
                &pairing_store,
                owner_bound_channels.contains(&ext.name),
            );
            ExtensionInfo {
                name: ext.name,
                display_name: ext.display_name,
                kind: ext.kind.to_string(),
                description: ext.description,
                url: ext.url,
                authenticated: ext.authenticated,
                active: ext.active,
                tools: ext.tools,
                needs_setup: ext.needs_setup,
                has_auth: ext.has_auth,
                derived: ext.derived,
                activation_status,
                activation_error: ext.activation_error,
                version: ext.version,
            }
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
    use std::fs;

    use tempfile::TempDir;

    use super::derive_activation_status;
    use crate::channels::web::types::ExtensionActivationStatus;
    use crate::extensions::{ExtensionKind, InstalledExtension};
    use crate::pairing::PairingStore;

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
            derived: false,
            installed: true,
            activation_error: None,
            version: None,
        }
    }

    #[test]
    fn active_authenticated_wasm_channel_without_allowlist_file_is_active() {
        let temp_dir = TempDir::new().expect("temp dir");
        let pairing_store = PairingStore::with_base_dir(temp_dir.path().to_path_buf());
        let ext = active_authenticated_wasm_channel("discord");

        assert_eq!(
            derive_activation_status(&ext, &pairing_store, false),
            Some(ExtensionActivationStatus::Active)
        );
    }

    #[test]
    fn active_authenticated_wasm_channel_with_empty_allowlist_file_is_pairing() {
        let temp_dir = TempDir::new().expect("temp dir");
        let pairing_store = PairingStore::with_base_dir(temp_dir.path().to_path_buf());
        let ext = active_authenticated_wasm_channel("discord");

        fs::write(
            temp_dir.path().join("discord-allowFrom.json"),
            r#"{"version":1,"allowFrom":[]}"#,
        )
        .expect("write empty allowlist");

        assert_eq!(
            derive_activation_status(&ext, &pairing_store, false),
            Some(ExtensionActivationStatus::Pairing)
        );
    }
}
