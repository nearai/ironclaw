use serde_json::{json, Map, Value};

use crate::cdp::CdpClient;
use crate::cdp_actions::{dispatch_cdp_action, send_session_command};
use crate::constants::*;
use crate::error::{DispatchFailure, DispatchSuccess, StructuredError};
use crate::near::agent::host as wit_host;

pub fn is_session_action(action: &str) -> bool {
    matches!(
        action,
        "session_create"
            | "session_list"
            | "session_resume"
            | "session_close"
            | "state_save"
            | "state_load"
    )
}

pub fn dispatch_session_action(
    action: &str,
    params: &Map<String, Value>,
    backend_url: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    let mut client = CdpClient::new();

    // session_create and session_resume use pooled connections so the
    // browser context survives across tool invocations.
    // session_list/session_close/state_save need a connection but can be
    // pooled too when a session_id is present.
    let session_id_param = params.get("session_id").and_then(Value::as_str);

    let conn = if let Some(sid) = session_id_param {
        CdpClient::connect_pooled(backend_url, sid)
    } else if action == "session_create" || action == "session_list" {
        // session_create generates its own id after connecting;
        // session_list doesn't need a persistent connection.
        CdpClient::connect(backend_url)
    } else {
        CdpClient::connect(backend_url)
    }
    .map_err(|e| DispatchFailure {
        error: StructuredError::new(ERR_NETWORK_FAILURE, e),
        attempts: 1,
    })?;

    let result = match action {
        "session_create" => session_create(&mut client, &conn, backend_url),
        "session_list" => session_list(&mut client, &conn),
        "session_resume" => session_resume(&mut client, &conn, params),
        "session_close" => session_close(&mut client, &conn, params),
        "state_save" => state_save(&mut client, &conn, params),
        "state_load" => state_load(params),
        _ => {
            return Err(DispatchFailure {
                error: StructuredError::new(
                    ERR_INVALID_ACTION,
                    format!("Unknown session action: {action}"),
                ),
                attempts: 1,
            });
        }
    };

    // Don't close pooled connections -- the pool manages their lifecycle.
    // Only close ephemeral (non-pooled) connections.
    if session_id_param.is_none() {
        let _ = conn.close();
    }

    result
}

/// Dispatch a page action using CDP through a session's persistent page.
/// If session_id is provided, uses a pooled WebSocket connection so the
/// browser context (and page state) persists across tool invocations.
/// If no session_id, creates an ephemeral context, runs the action, disposes.
pub fn dispatch_page_action(
    action: &str,
    params: &Map<String, Value>,
    backend_url: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    let mut client = CdpClient::new();
    let session_id_param = params.get("session_id").and_then(Value::as_str);

    if let Some(session_id) = session_id_param {
        // Pooled connection: browser context survives across invocations
        let conn =
            CdpClient::connect_pooled(backend_url, session_id).map_err(|e| DispatchFailure {
                error: StructuredError::new(ERR_NETWORK_FAILURE, e),
                attempts: 1,
            })?;

        let first = dispatch_with_session(&mut client, &conn, action, params, session_id);
        match first {
            Ok(success) => Ok(success),
            Err(failure) if should_recover_session(&failure.error) => {
                let fresh_conn =
                    CdpClient::connect_pooled(backend_url, session_id).map_err(|e| {
                        DispatchFailure {
                            error: StructuredError::new(ERR_NETWORK_FAILURE, e),
                            attempts: 1,
                        }
                    })?;
                dispatch_with_session(&mut client, &fresh_conn, action, params, session_id)
            }
            Err(failure) => Err(failure),
        }
        // Don't close -- the pool manages the connection lifecycle.
    } else {
        // Ephemeral connection: fresh context per call
        let conn = CdpClient::connect(backend_url).map_err(|e| DispatchFailure {
            error: StructuredError::new(ERR_NETWORK_FAILURE, e),
            attempts: 1,
        })?;

        let result = dispatch_ephemeral(&mut client, &conn, action, params, backend_url);
        let _ = conn.close();
        result
    }
}

fn error_looks_like_closed_stream(error: &StructuredError) -> bool {
    let msg = error.message.to_ascii_lowercase();
    msg.contains("websocket stream ended")
        || msg.contains("stream ended")
        || msg.contains("connection closed")
        || msg.contains("cdp recv failed")
}

fn should_recover_session(error: &StructuredError) -> bool {
    error.code == ERR_NETWORK_FAILURE && error_looks_like_closed_stream(error)
}

fn reopen_last_url_after_recovery(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    cdp_session: &str,
    last_url: &str,
) {
    let _ = dispatch_cdp_action(
        "open",
        &Map::from_iter([("url".to_string(), Value::String(last_url.to_string()))]),
        client,
        conn,
        None,
        cdp_session,
    );
}

fn dispatch_with_session(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    action: &str,
    params: &Map<String, Value>,
    session_id: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    // Load session state from workspace
    let workspace_path = format!("browser-sessions/{}.json", session_id);
    let session_json =
        wit_host::workspace_read(&workspace_path).ok_or_else(|| DispatchFailure {
            error: StructuredError::new(
                ERR_SESSION_NOT_FOUND,
                format!("Session {} not found in workspace", session_id),
            )
            .with_hint("Call session_create first to create a new session."),
            attempts: 1,
        })?;

    let session_state: Value =
        serde_json::from_str(&session_json).map_err(|e| DispatchFailure {
            error: StructuredError::new(
                ERR_SESSION_RESTORE_FAILED,
                format!("Invalid session data: {e}"),
            ),
            attempts: 1,
        })?;

    // With pooled connections the browser context persists across invocations.
    // Try to reuse the existing CDP session from workspace state.
    let cdp_session_id = session_state
        .get("cdpSessionId")
        .and_then(Value::as_str)
        .map(String::from);

    let stored_target_id = session_state
        .get("targetId")
        .and_then(Value::as_str)
        .map(String::from);

    let mut recovered_session = false;
    let cdp_session = if let Some(ref existing_session) = cdp_session_id {
        // Verify the page target is still alive
        let target_alive = if let Some(ref tid) = stored_target_id {
            client
                .get_targets(conn)
                .map(|targets| {
                    targets
                        .iter()
                        .any(|t| t.get("targetId").and_then(Value::as_str) == Some(tid))
                })
                .unwrap_or(false)
        } else {
            false
        };

        if target_alive {
            existing_session.clone()
        } else {
            // Target gone -- recreate context and page
            let (new_ctx, new_target, new_session) =
                create_fresh_session_state(client, conn, session_id)?;
            update_session_workspace(session_id, &new_ctx, &new_target, &new_session);
            recovered_session = true;
            new_session
        }
    } else {
        // No CDP session in state -- first call, create context + page
        let (new_ctx, new_target, new_session) =
            create_fresh_session_state(client, conn, session_id)?;
        update_session_workspace(session_id, &new_ctx, &new_target, &new_session);
        recovered_session = true;
        new_session
    };

    if recovered_session && action != "open" {
        let last_url = session_state
            .get("lastUrl")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| !url.is_empty() && *url != "about:blank");
        if let Some(url) = last_url {
            reopen_last_url_after_recovery(client, conn, &cdp_session, url);
        }
    }

    let action_result =
        dispatch_cdp_action(action, params, client, conn, Some(session_id), &cdp_session);

    if action == "open" {
        if let Ok(success) = &action_result {
            if let Some(url) = success.data.get("url").and_then(Value::as_str) {
                if let Some(mut state_obj) = session_state.as_object().cloned() {
                    state_obj.insert("lastUrl".to_string(), Value::String(url.to_string()));
                    state_obj.insert("updatedAt".to_string(), json!(wit_host::now_millis()));
                    let workspace_path = format!("browser-sessions/{}.json", session_id);
                    let _ = wit_host::workspace_write(
                        &workspace_path,
                        &Value::Object(state_obj).to_string(),
                    );
                }
            }
        }
    }

    action_result
    // Don't dispose context -- pooled connection keeps it alive for next call.
}

fn dispatch_ephemeral(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    action: &str,
    params: &Map<String, Value>,
    _backend_url: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    // Create ephemeral browser context + page
    let context_id = client
        .create_browser_context(conn)
        .map_err(|e| DispatchFailure {
            error: StructuredError::new(
                ERR_NETWORK_FAILURE,
                format!("Failed to create context: {e}"),
            ),
            attempts: 1,
        })?;

    let (target_id, cdp_session_id) = create_page_target(client, conn, &context_id)?;

    let result = dispatch_cdp_action(action, params, client, conn, None, &cdp_session_id);

    // Clean up: close the target and dispose context
    let _ = client.send_command(
        conn,
        "Target.closeTarget",
        Some(json!({ "targetId": target_id })),
    );
    let _ = client.dispose_browser_context(conn, &context_id);

    result
}

/// Create a fresh browser context + page for a session on a pooled connection.
/// Returns (context_id, target_id, cdp_session_id).
fn create_fresh_session_state(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    _session_id: &str,
) -> Result<(String, String, String), DispatchFailure> {
    let context_id = client
        .create_browser_context(conn)
        .map_err(|e| DispatchFailure {
            error: StructuredError::new(
                ERR_NETWORK_FAILURE,
                format!("Failed to create context: {e}"),
            ),
            attempts: 1,
        })?;

    let (target_id, cdp_session_id) = create_page_target(client, conn, &context_id)?;
    Ok((context_id, target_id, cdp_session_id))
}

/// Persist updated CDP IDs to workspace so the next invocation can reuse them.
fn update_session_workspace(
    session_id: &str,
    context_id: &str,
    target_id: &str,
    cdp_session_id: &str,
) {
    let workspace_path = format!("browser-sessions/{}.json", session_id);
    // Read existing state, update the CDP fields, write back
    let mut state: Value = wit_host::workspace_read(&workspace_path)
        .and_then(|d| serde_json::from_str(&d).ok())
        .unwrap_or_else(|| json!({"sessionId": session_id}));

    if let Some(obj) = state.as_object_mut() {
        obj.insert(
            "browserContextId".to_string(),
            Value::String(context_id.to_string()),
        );
        obj.insert("targetId".to_string(), Value::String(target_id.to_string()));
        obj.insert(
            "cdpSessionId".to_string(),
            Value::String(cdp_session_id.to_string()),
        );
        obj.insert("updatedAt".to_string(), json!(wit_host::now_millis()));
    }

    let _ = wit_host::workspace_write(&workspace_path, &state.to_string());
}

fn create_page_target(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    context_id: &str,
) -> Result<(String, String), DispatchFailure> {
    let create_result = client
        .send_command(
            conn,
            "Target.createTarget",
            Some(json!({
                "url": "about:blank",
                "browserContextId": context_id,
            })),
        )
        .map_err(|e| DispatchFailure {
            error: StructuredError::new(
                ERR_NETWORK_FAILURE,
                format!("Failed to create target: {e}"),
            ),
            attempts: 1,
        })?;

    let target_id = create_result
        .get("result")
        .and_then(|r| r.get("targetId"))
        .and_then(Value::as_str)
        .or_else(|| create_result.get("targetId").and_then(Value::as_str))
        .ok_or_else(|| DispatchFailure {
            error: StructuredError::new(
                ERR_NETWORK_FAILURE,
                "Missing targetId in createTarget response",
            ),
            attempts: 1,
        })?
        .to_string();

    let attach_result = client
        .send_command(
            conn,
            "Target.attachToTarget",
            Some(json!({
                "targetId": target_id,
                "flatten": true,
            })),
        )
        .map_err(|e| DispatchFailure {
            error: StructuredError::new(
                ERR_NETWORK_FAILURE,
                format!("Failed to attach to target: {e}"),
            ),
            attempts: 1,
        })?;

    let cdp_session_id = attach_result
        .get("result")
        .and_then(|r| r.get("sessionId"))
        .and_then(Value::as_str)
        .or_else(|| attach_result.get("sessionId").and_then(Value::as_str))
        .ok_or_else(|| DispatchFailure {
            error: StructuredError::new(
                ERR_NETWORK_FAILURE,
                "Missing sessionId in attachToTarget response",
            ),
            attempts: 1,
        })?
        .to_string();

    // Configure stealth settings so sites don't detect headless Chrome.
    // This must happen before any navigation.
    configure_stealth(client, conn, &cdp_session_id);

    Ok((target_id, cdp_session_id))
}

/// Apply stealth settings to hide headless Chrome indicators.
/// Overrides the user agent (removes "HeadlessChrome") and sets a realistic
/// desktop viewport. Non-fatal: failures are silently ignored.
fn configure_stealth(client: &mut CdpClient, conn: &wit_host::WsConnection, cdp_session_id: &str) {
    // Replace "HeadlessChrome" with "Chrome" in the user agent
    let _ = send_session_command(
        client,
        conn,
        cdp_session_id,
        "Network.setUserAgentOverride",
        Some(json!({
            "userAgent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36",
            "acceptLanguage": "en-US,en;q=0.9",
            "platform": "Linux"
        })),
    );

    // Set a desktop viewport (1280x800) so sites render full layouts
    let _ = send_session_command(
        client,
        conn,
        cdp_session_id,
        "Emulation.setDeviceMetricsOverride",
        Some(json!({
            "width": 1280,
            "height": 800,
            "deviceScaleFactor": 1,
            "mobile": false
        })),
    );

    // Hide webdriver flag
    let _ = send_session_command(
        client,
        conn,
        cdp_session_id,
        "Page.addScriptToEvaluateOnNewDocument",
        Some(json!({
            "source": "Object.defineProperty(navigator, 'webdriver', { get: () => false });"
        })),
    );
}

fn session_create(
    client: &mut CdpClient,
    _conn: &wit_host::WsConnection,
    backend_url: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    let session_id = crate::cdp::generate_session_id();

    // Open a POOLED connection keyed by the new session_id.
    // This ensures the browser context created below survives across invocations.
    // Falls back to the existing ephemeral connection if pooling is not available.
    let pooled_conn = CdpClient::connect_pooled(backend_url, &session_id);
    let use_conn: &wit_host::WsConnection;
    let pooled;

    match pooled_conn {
        Ok(ref c) => {
            pooled = true;
            use_conn = c;
        }
        Err(_) => {
            // Pooling not available -- fall back to ephemeral connection
            pooled = false;
            use_conn = _conn;
        }
    }

    let context_id = client
        .create_browser_context(use_conn)
        .map_err(|e| DispatchFailure {
            error: StructuredError::new(ERR_SESSION_RESTORE_FAILED, e),
            attempts: 1,
        })?;

    let (target_id, cdp_session_id) = create_page_target(client, use_conn, &context_id)?;

    let session_state = json!({
        "sessionId": session_id,
        "browserContextId": context_id,
        "targetId": target_id,
        "cdpSessionId": cdp_session_id,
        "createdAt": wit_host::now_millis(),
        "backendUrl": backend_url,
        "pooled": pooled,
    });

    let workspace_path = format!("browser-sessions/{}.json", session_id);
    let persistent = wit_host::workspace_write(&workspace_path, &session_state.to_string()).is_ok();

    Ok(DispatchSuccess {
        data: json!({
            "sessionId": session_id,
            "browserContextId": context_id,
            "targetId": target_id,
            "persistent": persistent,
            "pooled": pooled,
            "persistenceMode": if persistent { "persistent" } else { "ephemeral" }
        }),
        session_id: Some(session_id),
        snapshot_id: None,
        attempts: 1,
        backend_status: 200,
        warnings: if !persistent {
            vec!["Session state could not be persisted to workspace".to_string()]
        } else {
            vec![]
        },
    })
}

fn session_list(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
) -> Result<DispatchSuccess, DispatchFailure> {
    let targets = client.get_targets(conn).map_err(|e| DispatchFailure {
        error: StructuredError::new(ERR_NETWORK_FAILURE, e),
        attempts: 1,
    })?;

    let sessions: Vec<Value> = targets
        .into_iter()
        .filter(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
        .map(|t| {
            json!({
                "targetId": t.get("targetId"),
                "url": t.get("url"),
                "title": t.get("title"),
                "browserContextId": t.get("browserContextId"),
            })
        })
        .collect();

    let count = sessions.len();
    Ok(DispatchSuccess {
        data: json!({ "sessions": sessions, "count": count }),
        session_id: None,
        snapshot_id: None,
        attempts: 1,
        backend_status: 200,
        warnings: vec![],
    })
}

fn session_resume(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let session_id = params
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DispatchFailure {
            error: StructuredError::new(ERR_INVALID_PARAMS, "Missing session_id"),
            attempts: 1,
        })?;

    let workspace_path = format!("browser-sessions/{}.json", session_id);
    let session_data = wit_host::workspace_read(&workspace_path);

    match session_data {
        Some(data) => {
            let state: Value = serde_json::from_str(&data).map_err(|e| DispatchFailure {
                error: StructuredError::new(
                    ERR_SESSION_RESTORE_FAILED,
                    format!("Invalid session data: {e}"),
                ),
                attempts: 1,
            })?;

            let browser_context_id = state
                .get("browserContextId")
                .and_then(|v| v.as_str())
                .ok_or_else(|| DispatchFailure {
                    error: StructuredError::new(
                        ERR_SESSION_RESTORE_FAILED,
                        "Missing browserContextId",
                    ),
                    attempts: 1,
                })?;

            let targets = client.get_targets(conn).map_err(|e| DispatchFailure {
                error: StructuredError::new(ERR_NETWORK_FAILURE, e),
                attempts: 1,
            })?;

            let found = targets.iter().any(|t| {
                t.get("browserContextId").and_then(|v| v.as_str()) == Some(browser_context_id)
            });

            if !found {
                return Err(DispatchFailure {
                    error: StructuredError::new(
                        ERR_SESSION_NOT_FOUND,
                        "Browser context no longer exists",
                    )
                    .with_hint("The session may have expired or been closed."),
                    attempts: 1,
                });
            }

            Ok(DispatchSuccess {
                data: json!({
                    "sessionId": session_id,
                    "browserContextId": browser_context_id,
                    "targetId": state.get("targetId"),
                    "cdpSessionId": state.get("cdpSessionId"),
                    "resumed": true
                }),
                session_id: Some(session_id.to_string()),
                snapshot_id: None,
                attempts: 1,
                backend_status: 200,
                warnings: vec![],
            })
        }
        None => Err(DispatchFailure {
            error: StructuredError::new(
                ERR_SESSION_NOT_FOUND,
                format!("Session {} not found in workspace", session_id),
            )
            .with_hint("Call session_create first to create a new session."),
            attempts: 1,
        }),
    }
}

fn session_close(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let session_id = params
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DispatchFailure {
            error: StructuredError::new(ERR_INVALID_PARAMS, "Missing session_id"),
            attempts: 1,
        })?;

    let workspace_path = format!("browser-sessions/{}.json", session_id);
    let session_data = wit_host::workspace_read(&workspace_path);

    if let Some(data) = session_data {
        if let Ok(state) = serde_json::from_str::<Value>(&data) {
            // Close the page target
            if let Some(target_id) = state.get("targetId").and_then(Value::as_str) {
                let _ = client.send_command(
                    conn,
                    "Target.closeTarget",
                    Some(json!({ "targetId": target_id })),
                );
            }
            // Dispose the browser context
            if let Some(context_id) = state.get("browserContextId").and_then(Value::as_str) {
                let _ = client.dispose_browser_context(conn, context_id);
            }
        }
    }

    let closed_state = json!({
        "sessionId": session_id,
        "closed": true,
        "closedAt": wit_host::now_millis(),
    });
    let _ = wit_host::workspace_write(&workspace_path, &closed_state.to_string());

    Ok(DispatchSuccess {
        data: json!({ "sessionId": session_id, "closed": true }),
        session_id: Some(session_id.to_string()),
        snapshot_id: None,
        attempts: 1,
        backend_status: 200,
        warnings: vec![],
    })
}

fn state_save(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let session_id = params
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DispatchFailure {
            error: StructuredError::new(ERR_INVALID_PARAMS, "Missing session_id"),
            attempts: 1,
        })?;

    let state_key = params
        .get("state_key")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let workspace_path = format!("browser-sessions/{}.json", session_id);
    let session_data = wit_host::workspace_read(&workspace_path);

    let mut state: Value = match session_data {
        Some(data) => serde_json::from_str(&data).map_err(|e| DispatchFailure {
            error: StructuredError::new(
                ERR_SESSION_RESTORE_FAILED,
                format!("Invalid session data: {e}"),
            ),
            attempts: 1,
        })?,
        None => {
            return Err(DispatchFailure {
                error: StructuredError::new(
                    ERR_SESSION_NOT_FOUND,
                    format!("Session {} not found", session_id),
                ),
                attempts: 1,
            });
        }
    };

    let targets = client.get_targets(conn).map_err(|e| DispatchFailure {
        error: StructuredError::new(ERR_NETWORK_FAILURE, e),
        attempts: 1,
    })?;

    let current_page = targets
        .first()
        .and_then(|t| t.get("url").and_then(|v| v.as_str()).map(String::from));

    if let Some(obj) = state.as_object_mut() {
        let saved_states = obj.entry("savedStates").or_insert_with(|| json!({}));
        if let Some(states_obj) = saved_states.as_object_mut() {
            states_obj.insert(
                state_key.to_string(),
                json!({
                    "savedAt": wit_host::now_millis(),
                    "url": current_page,
                }),
            );
        }
        obj.insert("updatedAt".to_string(), json!(wit_host::now_millis()));
    }

    let _ = wit_host::workspace_write(&workspace_path, &state.to_string());

    Ok(DispatchSuccess {
        data: json!({
            "sessionId": session_id,
            "stateKey": state_key,
            "saved": true
        }),
        session_id: Some(session_id.to_string()),
        snapshot_id: None,
        attempts: 1,
        backend_status: 200,
        warnings: vec![],
    })
}

fn state_load(params: &Map<String, Value>) -> Result<DispatchSuccess, DispatchFailure> {
    let session_id = params
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DispatchFailure {
            error: StructuredError::new(ERR_INVALID_PARAMS, "Missing session_id"),
            attempts: 1,
        })?;

    let state_key = params
        .get("state_key")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let workspace_path = format!("browser-sessions/{}.json", session_id);
    let session_data = wit_host::workspace_read(&workspace_path);

    let state: Value = match session_data {
        Some(data) => serde_json::from_str(&data).map_err(|e| DispatchFailure {
            error: StructuredError::new(
                ERR_SESSION_RESTORE_FAILED,
                format!("Invalid session data: {e}"),
            ),
            attempts: 1,
        })?,
        None => {
            return Err(DispatchFailure {
                error: StructuredError::new(
                    ERR_SESSION_NOT_FOUND,
                    format!("Session {} not found", session_id),
                ),
                attempts: 1,
            });
        }
    };

    match state
        .get("savedStates")
        .and_then(|s| s.get(state_key))
        .cloned()
    {
        Some(saved) => Ok(DispatchSuccess {
            data: json!({
                "sessionId": session_id,
                "stateKey": state_key,
                "loaded": true,
                "state": saved
            }),
            session_id: Some(session_id.to_string()),
            snapshot_id: None,
            attempts: 1,
            backend_status: 200,
            warnings: vec![],
        }),
        None => Err(DispatchFailure {
            error: StructuredError::new(
                ERR_ARTIFACT_NOT_FOUND,
                format!("State key '{}' not found", state_key),
            ),
            attempts: 1,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_recover_session_for_stream_ended_error() {
        let err = StructuredError::new(ERR_NETWORK_FAILURE, "WebSocket stream ended while waiting");
        assert!(should_recover_session(&err));
    }

    #[test]
    fn test_should_not_recover_session_for_non_network_error() {
        let err = StructuredError::new(ERR_INVALID_PARAMS, "Missing selector");
        assert!(!should_recover_session(&err));
    }

    #[test]
    fn test_should_not_recover_session_for_other_network_error() {
        let err = StructuredError::new(ERR_NETWORK_FAILURE, "DNS lookup failed");
        assert!(!should_recover_session(&err));
    }
}
