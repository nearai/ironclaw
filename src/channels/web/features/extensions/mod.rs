//! Extension management API handlers.
//!
//! Owns the browser-facing extension lifecycle surface: list / readiness /
//! tools / install / activate / remove / registry / setup / setup-submit.
//! Migrated from `server.rs` in ironclaw#2599 stage 4d (final feature
//! slice before the `server.rs` shim can be retired).
//!
//! # Identity boundary
//!
//! Every handler that takes an extension name from the URL path validates
//! it through [`ironclaw_common::ExtensionName::new`] before the value
//! reaches extension lookup, SSE broadcast, or any `from_trusted` wrap.
//! Path-traversal / malformed slugs return 400 at the boundary. The rule
//! is enforced by check #8 in `scripts/pre-commit-safety.sh`; see the
//! "Identity types at the web boundary" section of
//! `src/channels/web/CLAUDE.md` for the full invariant.
//!
//! Auth-flow identity resolution (the pending-gate → extension-name map)
//! routes through the canonical
//! `AuthManager::resolve_extension_name_for_auth_flow` — `features/chat/`
//! owns the one wrapper. This slice does not re-derive extension names
//! from credential-name format strings or pending-gate fields.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use uuid::Uuid;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::platform::state::GatewayState;
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

/// Derive onboarding state and info from the activation status.
/// Returns `(None, None)` when the channel is not in a pairing state.
pub(crate) fn derive_onboarding(
    channel_name: &str,
    activation_status: Option<ExtensionActivationStatus>,
) -> (
    Option<ChannelOnboardingState>,
    Option<ChannelOnboardingInfo>,
) {
    match activation_status {
        Some(ExtensionActivationStatus::Pairing) => {
            // `channel_name` is the registry-sourced `Extension.name`,
            // which already passed validation at install time — no
            // re-sanitization needed.
            let state = ChannelOnboardingState::PairingRequired;
            let info = ChannelOnboardingInfo {
                state,
                requires_pairing: true,
                credential_title: None,
                credential_instructions: None,
                credential_next_step: None,
                setup_url: None,
                pairing_title: Some(format!("Claim ownership for {channel_name}")),
                pairing_instructions: Some(format!(
                    "Send a message to your {channel_name} bot, then paste the pairing code here."
                )),
                restart_instructions: Some(format!(
                    "To generate a new code, send another message to {channel_name}."
                )),
            };
            (Some(state), Some(info))
        }
        _ => (None, None),
    }
}

pub(crate) async fn extensions_list_handler(
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
            let (onboarding_state, onboarding) = derive_onboarding(&ext.name, activation_status);
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
                onboarding_state,
                onboarding,
            }
        })
        .collect();

    Ok(Json(ExtensionListResponse { extensions }))
}

pub(crate) fn extension_phase_for_web(
    ext: &crate::extensions::InstalledExtension,
) -> crate::extensions::ExtensionPhase {
    if ext.activation_error.is_some() {
        crate::extensions::ExtensionPhase::Error
    } else if ext.needs_setup {
        crate::extensions::ExtensionPhase::NeedsSetup
    } else if ext.has_auth && !ext.authenticated {
        crate::extensions::ExtensionPhase::NeedsAuth
    } else if ext.active
        || matches!(
            ext.kind,
            crate::extensions::ExtensionKind::WasmChannel
                | crate::extensions::ExtensionKind::ChannelRelay
        )
    {
        crate::extensions::ExtensionPhase::Ready
    } else {
        crate::extensions::ExtensionPhase::NeedsActivation
    }
}

pub(crate) async fn extensions_readiness_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<ExtensionReadinessResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let installed = ext_mgr
        .list(None, false, &user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let extensions = installed
        .into_iter()
        .map(|ext| {
            let phase = match extension_phase_for_web(&ext) {
                crate::extensions::ExtensionPhase::Installed => "installed",
                crate::extensions::ExtensionPhase::NeedsSetup => "needs_setup",
                crate::extensions::ExtensionPhase::NeedsAuth => "needs_auth",
                crate::extensions::ExtensionPhase::NeedsActivation => "needs_activation",
                crate::extensions::ExtensionPhase::Activating => "activating",
                crate::extensions::ExtensionPhase::Ready => "ready",
                crate::extensions::ExtensionPhase::Error => "error",
            }
            .to_string();
            ExtensionReadinessInfo {
                name: ext.name,
                kind: ext.kind.to_string(),
                phase,
                authenticated: ext.authenticated,
                active: ext.active,
                activation_error: ext.activation_error,
            }
        })
        .collect();

    Ok(Json(ExtensionReadinessResponse { extensions }))
}

pub(crate) async fn extensions_tools_handler(
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

pub(crate) async fn extensions_install_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<InstallExtensionRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // Validate the JSON-body `name` at the boundary — same rule the four
    // URL-path handlers (`activate`, `remove`, `setup`, `setup_submit`)
    // already enforce. Rejects path-traversal, invalid characters, and
    // malformed slugs with a 400 before the value reaches registry
    // lookup, filesystem path construction under `~/.ironclaw/extensions/`,
    // or any downstream extension-manager call. The canonical form
    // (hyphens folded to underscores) is used everywhere the previous
    // raw `req.name` was read, keeping the error messages and registry
    // lookup keyed off the same identity the install pipeline sees.
    let name = ironclaw_common::ExtensionName::new(&req.name).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid extension name: {e}"),
        )
    })?;
    let name_str = name.as_str();

    // When extension manager isn't available, check registry entries for a helpful message
    let Some(ext_mgr) = state.extension_manager.as_ref() else {
        // Look up the entry in the catalog to give a specific error
        if let Some(entry) = state.registry_entries.iter().find(|e| e.name == name_str) {
            let msg = match &entry.source {
                crate::extensions::ExtensionSource::WasmBuildable { .. } => {
                    format!(
                        "'{name_str}' requires building from source. \
                         Run `ironclaw registry install {name_str}` from the CLI."
                    )
                }
                _ => format!(
                    "Extension manager not available (secrets store required). \
                     Configure DATABASE_URL or a secrets backend to enable installation of '{name_str}'."
                ),
            };
            return Ok(Json(ActionResponse::fail(msg)));
        }
        return Ok(Json(ActionResponse::fail(
            "Extension manager not available (secrets store required)".to_string(),
        )));
    };

    let kind_hint = req.kind.as_deref().and_then(|k| match k {
        "mcp_server" => Some(crate::extensions::ExtensionKind::McpServer),
        "wasm_tool" => Some(crate::extensions::ExtensionKind::WasmTool),
        "wasm_channel" => Some(crate::extensions::ExtensionKind::WasmChannel),
        "channel_relay" => Some(crate::extensions::ExtensionKind::ChannelRelay),
        "acp_agent" => Some(crate::extensions::ExtensionKind::AcpAgent),
        _ => None,
    });

    match ext_mgr
        .install(name_str, req.url.as_deref(), kind_hint, &user.user_id)
        .await
    {
        Ok(result) => {
            let mut resp = ActionResponse::ok(result.message);
            match ext_mgr
                .ensure_extension_ready(
                    name_str,
                    &user.user_id,
                    crate::extensions::EnsureReadyIntent::PostInstall,
                )
                .await
            {
                Ok(readiness) => apply_extension_readiness_to_response(&mut resp, readiness, true),
                Err(e) => {
                    tracing::debug!(
                        extension = %name_str,
                        error = %e,
                        "Post-install readiness follow-through failed"
                    );
                }
            }

            Ok(Json(resp))
        }
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

pub(crate) fn apply_extension_readiness_to_response(
    resp: &mut ActionResponse,
    readiness: crate::extensions::EnsureReadyOutcome,
    preserve_success: bool,
) {
    match readiness {
        crate::extensions::EnsureReadyOutcome::Ready { activation, .. } => {
            if let Some(activation) = activation {
                resp.message = activation.message;
                resp.activated = Some(true);
            }
        }
        crate::extensions::EnsureReadyOutcome::NeedsAuth { auth, .. } => {
            let fallback = format!("'{}' requires authentication.", auth.name);
            if !preserve_success {
                resp.success = false;
                resp.message = auth
                    .instructions()
                    .map(String::from)
                    .unwrap_or_else(|| fallback.clone());
            } else if let Some(instructions) = auth.instructions() {
                resp.message = format!("{}. {}", resp.message, instructions);
            }
            resp.auth_url = auth.auth_url().map(String::from);
            resp.awaiting_token = Some(auth.is_awaiting_token());
            resp.instructions = auth.instructions().map(String::from);
        }
        crate::extensions::EnsureReadyOutcome::NeedsSetup {
            instructions,
            setup_url,
            ..
        } => {
            if !preserve_success {
                resp.success = false;
                resp.message = instructions.clone();
            } else {
                resp.message = format!("{}. {}", resp.message, instructions);
            }
            resp.instructions = Some(instructions);
            resp.auth_url = setup_url;
        }
    }
}

pub(crate) async fn extensions_activate_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // The URL path segment is user input — validate at the boundary via
    // `ExtensionName::new` and use the canonical form for all downstream
    // extension-manager calls and response formatting.
    let name = ironclaw_common::ExtensionName::new(&name).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid extension name: {e}"),
        )
    })?;
    tracing::trace!(
        extension = %name.as_str(),
        user_id = %user.user_id,
        "extensions_activate_handler: received activate request"
    );
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr
        .ensure_extension_ready(
            name.as_str(),
            &user.user_id,
            crate::extensions::EnsureReadyIntent::ExplicitActivate,
        )
        .await
    {
        Ok(readiness) => {
            let mut resp = ActionResponse::ok(format!("Extension '{}' is ready.", name.as_str()));
            apply_extension_readiness_to_response(&mut resp, readiness, false);
            Ok(Json(resp))
        }
        Err(err) => Ok(Json(ActionResponse::fail(err.to_string()))),
    }
}

pub(crate) async fn extensions_remove_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // Validate user-controlled path segment before it reaches the extension
    // manager — rejects path-traversal, invalid characters, and malformed
    // slugs with a 400.
    let name = ironclaw_common::ExtensionName::new(&name).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid extension name: {e}"),
        )
    })?;
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.remove(name.as_str(), &user.user_id).await {
        Ok(message) => Ok(Json(ActionResponse::ok(message))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

pub(crate) async fn extensions_registry_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(params): Query<RegistrySearchQuery>,
) -> Json<RegistrySearchResponse> {
    let query = params.query.unwrap_or_default();
    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();

    // Filter registry entries by query (or return all if empty)
    let matching: Vec<&crate::extensions::RegistryEntry> = if tokens.is_empty() {
        state.registry_entries.iter().collect()
    } else {
        state
            .registry_entries
            .iter()
            .filter(|e| {
                let name = e.name.to_lowercase();
                let display = e.display_name.to_lowercase();
                let desc = e.description.to_lowercase();
                tokens.iter().any(|t| {
                    name.contains(t)
                        || display.contains(t)
                        || desc.contains(t)
                        || e.keywords.iter().any(|k| k.to_lowercase().contains(t))
                })
            })
            .collect()
    };

    // Cross-reference with installed extensions by (name, kind) to avoid
    // false positives when the same name exists as different kinds.
    let installed: std::collections::HashSet<(String, String)> =
        if let Some(ext_mgr) = state.extension_manager.as_ref() {
            ext_mgr
                .list(None, false, &user.user_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|ext| (ext.name, ext.kind.to_string()))
                .collect()
        } else {
            std::collections::HashSet::new()
        };

    let entries = matching
        .into_iter()
        .map(|e| {
            let kind_str = e.kind.to_string();
            RegistryEntryInfo {
                name: e.name.clone(),
                display_name: e.display_name.clone(),
                installed: installed.contains(&(e.name.clone(), kind_str.clone())),
                kind: kind_str,
                description: e.description.clone(),
                keywords: e.keywords.clone(),
                version: e.version.clone(),
            }
        })
        .collect();

    Json(RegistrySearchResponse { entries })
}

pub(crate) async fn extensions_setup_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
) -> Result<Json<ExtensionSetupResponse>, (StatusCode, String)> {
    // Validate user-controlled path segment at entry. Downstream lookups
    // (`get_setup_schema`, `list().find(...)`) consume the canonical form.
    let name = ironclaw_common::ExtensionName::new(&name).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid extension name: {e}"),
        )
    })?;
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let setup = ext_mgr
        .get_setup_schema(name.as_str(), &user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let kind = ext_mgr
        .list(None, false, &user.user_id)
        .await
        .ok()
        .and_then(|list| list.into_iter().find(|e| e.name == name.as_str()))
        .map(|e| e.kind.to_string())
        .unwrap_or_default();

    Ok(Json(ExtensionSetupResponse {
        name: name.as_str().to_string(),
        kind,
        secrets: setup.secrets,
        fields: setup.fields,
        onboarding_state: None,
        onboarding: None,
    }))
}

pub(crate) async fn extensions_setup_submit_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
    Json(req): Json<ExtensionSetupRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    // The URL path segment is user input — validate at the boundary via
    // `ExtensionName::new`. Reject path-traversal, invalid characters, or
    // malformed slugs with a 400 before the value reaches extension
    // lookup, SSE broadcast, or any `from_trusted` wrap below.
    let name = ironclaw_common::ExtensionName::new(&name).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid extension name: {e}"),
        )
    })?;

    // Clear auth mode regardless of outcome so the next user message goes
    // through to the LLM instead of being intercepted as a token.
    crate::channels::web::platform::legacy_auth::clear_auth_mode(&state, &user.user_id).await;

    match ext_mgr
        .configure(name.as_str(), &req.secrets, &req.fields, &user.user_id)
        .await
    {
        Ok(result) => {
            // Return ok when activated OR when an OAuth auth_url is present
            // (activation is expected to be false until OAuth completes).
            let mut resp = if result.activated || result.auth_url.is_some() {
                ActionResponse::ok(result.message.clone())
            } else {
                ActionResponse::fail(result.message.clone())
            };
            resp.activated = Some(result.activated);
            resp.auth_url = result.auth_url.clone();
            resp.onboarding_state = result.onboarding_state;
            resp.onboarding = result.onboarding.clone();
            let outcome = crate::channels::web::onboarding::classify_configure_result(&result);
            let mut onboarding_event =
                crate::channels::web::onboarding::event_from_configure_result(
                    name.clone(),
                    &result,
                    req.thread_id.clone(),
                );
            if let (Some(request_id), Some(thread_id)) =
                (req.request_id.as_deref(), req.thread_id.as_deref())
            {
                match outcome {
                    crate::channels::web::onboarding::ConfigureFlowOutcome::AuthRequired => {}
                    crate::channels::web::onboarding::ConfigureFlowOutcome::PairingRequired {
                        instructions,
                        onboarding,
                    } => {
                        let request_id = Uuid::parse_str(request_id).map_err(|_| {
                            (
                                StatusCode::BAD_REQUEST,
                                "Invalid request_id (expected UUID)".to_string(),
                            )
                        })?;
                        if let Some(next_request_id) =
                            crate::bridge::transition_engine_pending_auth_request_to_pairing(
                                &user.user_id,
                                request_id,
                                Some(thread_id),
                                name.as_str(),
                            )
                            .await
                            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                        {
                            onboarding_event =
                                crate::channels::web::types::OnboardingStateDto::pairing_required(
                                    name.clone(),
                                    Some(next_request_id),
                                    Some(thread_id.to_string()),
                                    Some(result.message.clone()),
                                    instructions,
                                    onboarding,
                                );
                        }
                    }
                    crate::channels::web::onboarding::ConfigureFlowOutcome::Ready => {
                        crate::channels::web::platform::engine_dispatch::dispatch_engine_external_callback(
                            &state,
                            &user.user_id,
                            thread_id,
                            request_id,
                        )
                        .await?;
                    }
                    crate::channels::web::onboarding::ConfigureFlowOutcome::RetryAuth => {}
                }
            }
            // Broadcast the canonical onboarding state so the chat UI can
            // dismiss or advance any in-progress onboarding UI.
            state
                .sse
                .broadcast_for_user(&user.user_id, onboarding_event);
            Ok(Json(resp))
        }
        Err(e) => {
            // Preserve the `activated` field on the failure path so clients
            // (and regression tests) see an explicit `false` rather than
            // `null`. `ActionResponse::fail` leaves `activated` as `None`,
            // which serializes to `null` and makes "did activation fail?"
            // ambiguous from the wire.
            let mut resp = ActionResponse::fail(e.to_string());
            resp.activated = Some(false);
            Ok(Json(resp))
        }
    }
}

#[cfg(test)]
mod tests {

    use axum::{
        Router,
        http::StatusCode,
        routing::{get, post},
    };

    use crate::channels::web::auth::UserIdentity;

    use crate::channels::web::features::extensions::{
        apply_extension_readiness_to_response, extension_phase_for_web,
        extensions_activate_handler, extensions_install_handler, extensions_list_handler,
        extensions_readiness_handler, extensions_remove_handler, extensions_setup_handler,
        extensions_setup_submit_handler,
    };

    use crate::channels::web::test_helpers::{
        test_ext_mgr, test_ext_mgr_with_db, test_gateway_state, test_secrets_store,
    };
    use crate::channels::web::types::*;
    use crate::channels::web::types::{
        ExtensionActivationStatus, classify_wasm_channel_activation,
    };
    use crate::extensions::{ExtensionKind, InstalledExtension};

    use super::{derive_activation_status, derive_onboarding};
    use crate::channels::web::types::ChannelOnboardingState;

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

    #[test]
    fn derive_onboarding_returns_pairing_required_for_pairing_status() {
        let (state, info) = derive_onboarding("telegram", Some(ExtensionActivationStatus::Pairing));
        assert_eq!(state, Some(ChannelOnboardingState::PairingRequired));
        let info = info.expect("onboarding info should be present");
        assert!(info.requires_pairing);
        assert!(info.pairing_title.unwrap().contains("telegram"));
        assert!(info.pairing_instructions.unwrap().contains("telegram"));
    }

    #[test]
    fn derive_onboarding_returns_none_for_non_pairing_status() {
        let (state, info) = derive_onboarding("telegram", Some(ExtensionActivationStatus::Active));
        assert!(state.is_none());
        assert!(info.is_none());

        let (state, info) = derive_onboarding("telegram", None);
        assert!(state.is_none());
        assert!(info.is_none());
    }

    #[test]
    fn test_wasm_channel_activation_status_owner_bound_counts_as_active() -> Result<(), String> {
        let ext = InstalledExtension {
            name: "telegram".to_string(),
            kind: ExtensionKind::WasmChannel,
            display_name: Some("Telegram".to_string()),
            description: None,
            url: None,
            authenticated: true,
            active: true,
            tools: Vec::new(),
            needs_setup: true,
            has_auth: false,
            installed: true,
            activation_error: None,
            version: None,
        };

        let owner_bound = classify_wasm_channel_activation(&ext, false, true);
        if owner_bound != Some(ExtensionActivationStatus::Active) {
            return Err(format!(
                "owner-bound channel should be active, got {:?}",
                owner_bound
            ));
        }

        let unbound = classify_wasm_channel_activation(&ext, false, false);
        if unbound != Some(ExtensionActivationStatus::Pairing) {
            return Err(format!(
                "unbound channel should be pairing, got {:?}",
                unbound
            ));
        }

        Ok(())
    }

    #[test]
    fn test_channel_relay_activation_status_is_preserved() -> Result<(), String> {
        let relay = InstalledExtension {
            name: "signal".to_string(),
            kind: ExtensionKind::ChannelRelay,
            display_name: Some("Signal".to_string()),
            description: None,
            url: None,
            authenticated: true,
            active: false,
            tools: Vec::new(),
            needs_setup: true,
            has_auth: false,
            installed: true,
            activation_error: None,
            version: None,
        };

        let status = if relay.kind == crate::extensions::ExtensionKind::WasmChannel {
            classify_wasm_channel_activation(&relay, false, false)
        } else if relay.kind == crate::extensions::ExtensionKind::ChannelRelay {
            Some(if relay.active {
                ExtensionActivationStatus::Active
            } else if relay.authenticated {
                ExtensionActivationStatus::Configured
            } else {
                ExtensionActivationStatus::Installed
            })
        } else {
            None
        };

        if status != Some(ExtensionActivationStatus::Configured) {
            return Err(format!(
                "channel relay should retain configured status, got {:?}",
                status
            ));
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_extensions_setup_submit_rejects_path_traversal_name() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets);

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route(
                "/api/extensions/{name}/setup",
                post(extensions_setup_submit_handler),
            )
            .with_state(state);

        // Each of these slugs would have silently reached extension lookup
        // under the old `from_trusted(name)` wrap. All must reject at 400.
        // We use axum::http::uri::PathAndQuery-safe escape where needed so
        // the path extractor still decodes into a valid `String`.
        for bad in [
            "..%2Ftraversal",
            "slash%2Fname",
            "BadCase",
            "has%20space",
            "trailing_",
        ] {
            let req_body = serde_json::json!({"secrets": {}});
            let mut req = axum::http::Request::builder()
                .method("POST")
                .uri(format!("/api/extensions/{bad}/setup"))
                .header("content-type", "application/json")
                .body(Body::from(req_body.to_string()))
                .expect("request");
            req.extensions_mut().insert(UserIdentity {
                user_id: "test".to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: Vec::new(),
            });

            let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), req)
                .await
                .expect("response");
            assert_eq!(
                resp.status(),
                StatusCode::BAD_REQUEST,
                "expected 400 for malformed extension name {bad:?}, got {:?}",
                resp.status()
            );
        }
    }

    #[tokio::test]
    async fn test_extensions_sibling_handlers_reject_path_traversal_name() {
        use axum::body::Body;
        use axum::routing::{get, post};
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets);

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route(
                "/api/extensions/{name}/activate",
                post(extensions_activate_handler),
            )
            .route(
                "/api/extensions/{name}/remove",
                post(extensions_remove_handler),
            )
            .route(
                "/api/extensions/{name}/setup",
                get(extensions_setup_handler),
            )
            .with_state(state);

        let bad_names = [
            "..%2Ftraversal",
            "slash%2Fname",
            "BadCase",
            "has%20space",
            "trailing_",
        ];
        let routes = [("POST", "activate"), ("POST", "remove"), ("GET", "setup")];

        for bad in bad_names {
            for (method, suffix) in routes {
                let mut builder = axum::http::Request::builder()
                    .method(method)
                    .uri(format!("/api/extensions/{bad}/{suffix}"));
                if method == "POST" {
                    builder = builder.header("content-type", "application/json");
                }
                let body = if method == "POST" {
                    Body::from("{}")
                } else {
                    Body::empty()
                };
                let mut req = builder.body(body).expect("request");
                req.extensions_mut().insert(UserIdentity {
                    user_id: "test".to_string(),
                    role: "admin".to_string(),
                    workspace_read_scopes: Vec::new(),
                });

                let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), req)
                    .await
                    .expect("response");
                assert_eq!(
                    resp.status(),
                    StatusCode::BAD_REQUEST,
                    "expected 400 for {method} {suffix} with malformed name {bad:?}, got {:?}",
                    resp.status()
                );
            }
        }
    }

    #[tokio::test]
    async fn test_extensions_install_handler_rejects_malformed_name() {
        use axum::body::Body;
        use axum::routing::post;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets);

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route("/api/extensions/install", post(extensions_install_handler))
            .with_state(state);

        // Each of these is the JSON-body `name` the handler now validates
        // through `ExtensionName::new`. Previously `req.name` was taken
        // verbatim into `ext_mgr.install(&req.name, ...)` which constructs
        // filesystem paths under `~/.ironclaw/extensions/` — so path-traversal
        // / separators / control characters could silently reach the
        // filesystem layer before failing deep in the install pipeline.
        for bad in ["..", "../traversal", "slash/name", "BadCase", "has space"] {
            let req_body = serde_json::json!({ "name": bad });
            let mut req = axum::http::Request::builder()
                .method("POST")
                .uri("/api/extensions/install")
                .header("content-type", "application/json")
                .body(Body::from(req_body.to_string()))
                .expect("request");
            req.extensions_mut().insert(UserIdentity {
                user_id: "test".to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: Vec::new(),
            });

            let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), req)
                .await
                .expect("response");
            assert_eq!(
                resp.status(),
                StatusCode::BAD_REQUEST,
                "expected 400 for install body.name {bad:?}, got {:?}",
                resp.status()
            );
        }
    }

    #[tokio::test]
    async fn test_extensions_setup_submit_returns_failure_when_not_activated() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, wasm_channels_dir) = test_ext_mgr(secrets);

        // Use underscore-only name: `canonicalize_extension_name` rewrites
        // hyphens to underscores, but `configure`'s capabilities-file lookup
        // does not fall back to the legacy hyphen form, so a hyphenated test
        // channel name causes `Capabilities file not found` and the handler
        // takes the `Err` branch (no `activated` field) instead of the
        // intended "saved but activation failed" branch.
        let channel_name = "test_failing_channel";
        std::fs::write(
            wasm_channels_dir
                .path()
                .join(format!("{channel_name}.wasm")),
            b"\0asm fake",
        )
        .expect("write fake wasm");
        let caps = serde_json::json!({
            "type": "channel",
            "name": channel_name,
            "setup": {
                "required_secrets": [
                    {"name": "BOT_TOKEN", "prompt": "Enter bot token"}
                ]
            }
        });
        std::fs::write(
            wasm_channels_dir
                .path()
                .join(format!("{channel_name}.capabilities.json")),
            serde_json::to_string(&caps).expect("serialize caps"),
        )
        .expect("write capabilities");

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route(
                "/api/extensions/{name}/setup",
                post(extensions_setup_submit_handler),
            )
            .with_state(state);

        let req_body = serde_json::json!({
            "secrets": {
                "BOT_TOKEN": "dummy-token"
            }
        });
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri(format!("/api/extensions/{channel_name}/setup"))
            .header("content-type", "application/json")
            .body(Body::from(req_body.to_string()))
            .expect("request");
        // Inject AuthenticatedUser so the handler's extractor succeeds
        // without needing the full auth middleware layer.
        req.extensions_mut().insert(UserIdentity {
            user_id: "test".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json response");
        assert_eq!(parsed["success"], serde_json::Value::Bool(false));
        assert_eq!(parsed["activated"], serde_json::Value::Bool(false));
        assert!(
            parsed["message"]
                .as_str()
                .unwrap_or_default()
                .contains("Activation failed"),
            "expected activation failure in message: {:?}",
            parsed
        );
    }

    #[tokio::test]
    async fn test_extensions_list_reports_installed_inactive_wasm_channel_as_inactive() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, wasm_channels_dir) = test_ext_mgr(secrets);
        let channel_name = "telegram";
        std::fs::write(
            wasm_channels_dir
                .path()
                .join(format!("{channel_name}.wasm")),
            b"\0asm fake",
        )
        .expect("write fake wasm");

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route("/api/extensions", get(extensions_list_handler))
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/extensions")
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json response");
        let telegram = parsed["extensions"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["name"] == channel_name))
            .expect("telegram extension entry");
        assert_eq!(telegram["kind"], "wasm_channel");
        assert_eq!(telegram["active"], false);
        assert_eq!(telegram["authenticated"], false);
        assert_eq!(telegram["activation_status"], "installed");
    }

    #[test]
    fn test_extension_phase_for_web_prefers_error_then_readiness() {
        let mut ext = crate::extensions::InstalledExtension {
            name: "notion".to_string(),
            kind: crate::extensions::ExtensionKind::McpServer,
            display_name: None,
            description: None,
            url: None,
            authenticated: false,
            active: false,
            tools: Vec::new(),
            needs_setup: false,
            has_auth: true,
            installed: true,
            activation_error: Some("boom".to_string()),
            version: None,
        };
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::Error
        );

        ext.activation_error = None;
        ext.needs_setup = true;
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::NeedsSetup
        );

        ext.needs_setup = false;
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::NeedsAuth
        );

        ext.authenticated = true;
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::NeedsActivation
        );

        ext.active = true;
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::Ready
        );
    }

    #[tokio::test]
    async fn test_extensions_readiness_handler_reports_phase_summary() {
        use axum::body::Body;
        use tower::ServiceExt;

        // DB-backed manager so the install path does not fall back to the
        // developer's real `~/.ironclaw/mcp-servers.json` (which would
        // panic with `AlreadyInstalled("notion")` on dev machines that
        // already have a notion entry configured).
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir, _db_dir) = test_ext_mgr_with_db().await;
        let mut server =
            crate::tools::mcp::McpServerConfig::new("notion", "https://mcp.notion.com/mcp");
        server.description = Some("Notion".to_string());
        ext_mgr
            .install(
                "notion",
                Some(&server.url),
                Some(crate::extensions::ExtensionKind::McpServer),
                "test",
            )
            .await
            .expect("install notion mcp");

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route(
                "/api/extensions/readiness",
                get(extensions_readiness_handler),
            )
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/extensions/readiness")
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json response");
        let notion = parsed["extensions"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["name"] == "notion"))
            .expect("notion readiness entry");
        assert_eq!(notion["kind"], "mcp_server");
        assert_eq!(notion["phase"], "needs_auth");
        assert_eq!(notion["authenticated"], false);
        assert_eq!(notion["active"], false);
    }

    #[tokio::test]
    async fn test_extensions_list_handler_reports_installed_inactive_wasm_channel_as_inactive() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, wasm_channels_dir) = test_ext_mgr(secrets);
        std::fs::write(wasm_channels_dir.path().join("telegram.wasm"), b"fake-wasm")
            .expect("write fake telegram wasm");
        std::fs::write(
            wasm_channels_dir.path().join("telegram.capabilities.json"),
            serde_json::json!({
                "type": "channel",
                "name": "telegram",
                "description": "Telegram",
                "capabilities": {
                    "channel": {
                        "allowed_paths": ["/webhook/telegram"]
                    }
                }
            })
            .to_string(),
        )
        .expect("write telegram capabilities");

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route("/api/extensions", get(extensions_list_handler))
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/extensions")
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json response");
        let telegram = parsed["extensions"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["name"] == "telegram"))
            .expect("telegram extensions entry");

        assert_eq!(telegram["kind"], "wasm_channel");
        assert_eq!(telegram["active"], false);
        assert_eq!(telegram["activation_status"], "installed");
    }

    /// Caller-level wire-contract regression for nearai/ironclaw#2235.
    ///
    /// The Settings → Extensions UI picks the WASM-channel fallback button
    /// label ("Setup" vs "Reconfigure") from `ExtensionInfo.authenticated`
    /// on the `/api/extensions` response. A backend regression that left
    /// `authenticated=false` after credentials were written — or dropped
    /// the field off the wire entirely — would silently re-show the
    /// credential popup on an already-configured install. The unit-level
    /// classifier tests above cannot catch this because they do not
    /// exercise the `configure()` → `list()` wire round-trip.
    ///
    /// This test drives the real pair of handlers — POST setup then GET
    /// list — against a stub channel whose WASM binary intentionally
    /// fails to activate (so `active=false`). The `authenticated` flag
    /// must still flip to `true` once the required secret lands in the
    /// secrets store.
    #[tokio::test]
    async fn test_extensions_list_reports_authenticated_after_setup_submit() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, wasm_channels_dir) = test_ext_mgr(secrets);

        let channel_name = "telegram";
        std::fs::write(
            wasm_channels_dir
                .path()
                .join(format!("{channel_name}.wasm")),
            b"\0asm fake",
        )
        .expect("write fake wasm");
        let caps = serde_json::json!({
            "type": "channel",
            "name": channel_name,
            "setup": {
                "required_secrets": [
                    {"name": "BOT_TOKEN", "prompt": "Enter bot token"}
                ]
            }
        });
        std::fs::write(
            wasm_channels_dir
                .path()
                .join(format!("{channel_name}.capabilities.json")),
            serde_json::to_string(&caps).expect("serialize caps"),
        )
        .expect("write capabilities");

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route("/api/extensions", get(extensions_list_handler))
            .route(
                "/api/extensions/{name}/setup",
                post(extensions_setup_submit_handler),
            )
            .with_state(state);

        // Pre-setup: authenticated must be false.
        let mut req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/extensions")
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), req)
            .await
            .expect("pre-setup response");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json");
        let telegram = parsed["extensions"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["name"] == channel_name))
            .expect("telegram entry pre-setup");
        assert_eq!(
            telegram["authenticated"], false,
            "pre-setup: authenticated must start false (the JS Settings card shows \
             'Setup' in this state — regressing it to true would flip the button to \
             'Reconfigure' before credentials exist, re-introducing #2235)"
        );

        // Submit credentials via the real setup-submit handler.
        let submit_body = serde_json::json!({
            "secrets": {
                "BOT_TOKEN": "dummy-token"
            }
        });
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri(format!("/api/extensions/{channel_name}/setup"))
            .header("content-type", "application/json")
            .body(Body::from(submit_body.to_string()))
            .expect("setup-submit request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), req)
            .await
            .expect("setup-submit response");
        assert_eq!(resp.status(), StatusCode::OK);

        // Post-setup: the wire contract the Settings UI depends on.
        let mut req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/extensions")
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("post-setup response");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json");
        let telegram = parsed["extensions"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["name"] == channel_name))
            .expect("telegram entry post-setup");
        // A missing field indexes to `Value::Null` and trips this assertion
        // too, so the single check covers both "field stripped from the wire"
        // and "field present but wrong value".
        assert_eq!(
            telegram["authenticated"], true,
            "post-setup: the `authenticated` flag must be present and true once \
             the required secret is written — the Settings card's \
             Setup/Reconfigure branch reads this field directly, and a regression \
             here reopens #2235."
        );
    }

    #[test]
    fn apply_extension_readiness_preserves_install_success_for_auth_followup() {
        let mut resp = ActionResponse::ok("Installed notion");
        apply_extension_readiness_to_response(
            &mut resp,
            crate::extensions::EnsureReadyOutcome::NeedsAuth {
                name: "notion".to_string(),
                kind: crate::extensions::ExtensionKind::McpServer,
                phase: crate::extensions::ExtensionPhase::NeedsAuth,
                credential_name: Some("notion_api_token".to_string()),
                auth: crate::extensions::AuthResult::awaiting_authorization(
                    "notion",
                    crate::extensions::ExtensionKind::McpServer,
                    "https://example.com/oauth".to_string(),
                    "gateway".to_string(),
                ),
            },
            true,
        );

        assert!(resp.success);
        assert_eq!(resp.auth_url.as_deref(), Some("https://example.com/oauth"));
        assert_eq!(resp.awaiting_token, Some(false));
    }

    #[test]
    fn apply_extension_readiness_fails_activate_when_auth_is_required() {
        let mut resp = ActionResponse::ok("placeholder");
        apply_extension_readiness_to_response(
            &mut resp,
            crate::extensions::EnsureReadyOutcome::NeedsAuth {
                name: "notion".to_string(),
                kind: crate::extensions::ExtensionKind::McpServer,
                phase: crate::extensions::ExtensionPhase::NeedsAuth,
                credential_name: Some("notion_api_token".to_string()),
                auth: crate::extensions::AuthResult::awaiting_token(
                    "notion",
                    crate::extensions::ExtensionKind::McpServer,
                    "Paste your Notion token".to_string(),
                    None,
                ),
            },
            false,
        );

        assert!(!resp.success);
        assert_eq!(resp.awaiting_token, Some(true));
        assert_eq!(
            resp.instructions.as_deref(),
            Some("Paste your Notion token")
        );
        assert_eq!(resp.message, "Paste your Notion token");
    }
}
