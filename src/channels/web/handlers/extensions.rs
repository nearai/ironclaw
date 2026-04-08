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

/// Derive the activation status for an installed extension.
///
/// `has_paired` reflects whether any sender has been paired against
/// `channel_identities` for this WASM channel (queried from the DB-backed
/// pairing store). `has_owner_binding` reflects whether the channel has an
/// explicit owner_id in settings. Either is sufficient to upgrade an active
/// channel from `Pairing` to `Active`.
///
/// See nearai/ironclaw#1921 for the regression that motivated plumbing
/// `has_paired` through here instead of hardcoding it to `false`.
pub(crate) fn derive_activation_status(
    ext: &crate::extensions::InstalledExtension,
    has_paired: bool,
    has_owner_binding: bool,
) -> Option<ExtensionActivationStatus> {
    if ext.kind == crate::extensions::ExtensionKind::WasmChannel {
        classify_wasm_channel_activation(ext, has_paired, has_owner_binding)
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

    let mut owner_bound_channels = std::collections::HashSet::new();
    let mut paired_channels = std::collections::HashSet::new();
    for ext in &installed {
        if ext.kind == crate::extensions::ExtensionKind::WasmChannel {
            if ext_mgr.has_wasm_channel_owner_binding(&ext.name).await {
                owner_bound_channels.insert(ext.name.clone());
            }
            if ext_mgr.has_wasm_channel_pairing(&ext.name).await {
                paired_channels.insert(ext.name.clone());
            }
        }
    }
    let extensions = installed
        .into_iter()
        .map(|ext| {
            let activation_status = derive_activation_status(
                &ext,
                paired_channels.contains(&ext.name),
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
                activation_status,
                activation_error: ext.activation_error,
                version: ext.version,
                onboarding_state: None,
                onboarding: None,
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
    use super::derive_activation_status;
    use crate::channels::web::types::ExtensionActivationStatus;
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

    /// Full truth table for an active+authenticated WASM channel.
    ///
    /// Either `has_paired` or `has_owner_binding` is sufficient to upgrade
    /// from `Pairing` to `Active`. Pinning the four-cell matrix here means a
    /// regression that drops one axis (the bug shape behind nearai/ironclaw#1921)
    /// trips at least two cells, not zero.
    #[test]
    fn derive_activation_status_truth_table_for_active_wasm_channel() {
        let ext = active_authenticated_wasm_channel("discord");
        let cases = [
            (false, false, ExtensionActivationStatus::Pairing),
            (false, true, ExtensionActivationStatus::Active),
            (true, false, ExtensionActivationStatus::Active),
            (true, true, ExtensionActivationStatus::Active),
        ];
        for (has_paired, has_owner_binding, expected) in cases {
            let actual = derive_activation_status(&ext, has_paired, has_owner_binding);
            assert_eq!(
                actual,
                Some(expected),
                "derive_activation_status(has_paired={has_paired}, \
                 has_owner_binding={has_owner_binding}) should be {:?}",
                expected
            );
        }
    }

    /// Regression for nearai/ironclaw#1921 — caller-level coverage.
    ///
    /// Before this fix the wrapper hardcoded the underlying classifier's
    /// `has_paired` axis to `false`, so a paired-but-not-owner-bound
    /// channel was misreported as `Pairing`. This test pins the case
    /// that would silently regress if a future refactor drops the
    /// `has_paired` argument.
    #[test]
    fn paired_wasm_channel_without_owner_binding_is_active() {
        let ext = active_authenticated_wasm_channel("discord");
        assert_eq!(
            derive_activation_status(&ext, true, false),
            Some(ExtensionActivationStatus::Active),
            "a WASM channel with paired senders must report Active even when \
             no owner binding is set (nearai/ironclaw#1921)"
        );
    }
}
