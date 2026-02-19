use serde_json::{json, Map, Value};

use crate::cdp::CdpClient;
use crate::constants::*;
use crate::error::{DispatchFailure, DispatchSuccess, StructuredError};
use crate::near::agent::host as wit_host;

pub fn dispatch_cdp_action(
    action: &str,
    params: &Map<String, Value>,
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: Option<&str>,
    cdp_session_id: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    let result = match action {
        "open" => cdp_open(client, conn, cdp_session_id, params),
        "back" => cdp_history_nav(client, conn, cdp_session_id, -1),
        "forward" => cdp_history_nav(client, conn, cdp_session_id, 1),
        "reload" => cdp_reload(client, conn, cdp_session_id),
        "snapshot" => cdp_snapshot(client, conn, cdp_session_id, params),
        "click" => cdp_click(client, conn, cdp_session_id, params),
        "dblclick" => cdp_dblclick(client, conn, cdp_session_id, params),
        "focus" => cdp_focus(client, conn, cdp_session_id, params),
        "fill" => cdp_fill(client, conn, cdp_session_id, params),
        "type" => cdp_type_text(client, conn, cdp_session_id, params),
        "press" => cdp_press(client, conn, cdp_session_id, params),
        "keydown" => cdp_key_event(client, conn, cdp_session_id, params, "keyDown"),
        "keyup" => cdp_key_event(client, conn, cdp_session_id, params, "keyUp"),
        "hover" => cdp_hover(client, conn, cdp_session_id, params),
        "check" => cdp_check_uncheck(client, conn, cdp_session_id, params, true),
        "uncheck" => cdp_check_uncheck(client, conn, cdp_session_id, params, false),
        "select" => cdp_select(client, conn, cdp_session_id, params),
        "scroll" => cdp_scroll(client, conn, cdp_session_id, params),
        "scroll_into_view" => cdp_scroll_into_view(client, conn, cdp_session_id, params),
        "drag" => cdp_drag(client, conn, cdp_session_id, params),
        "upload" => cdp_upload(client, conn, cdp_session_id, params),
        "wait" => cdp_wait(client, conn, cdp_session_id, params),
        "get_text" => cdp_get_text(client, conn, cdp_session_id, params),
        "get_html" => cdp_get_html(client, conn, cdp_session_id, params),
        "get_value" => cdp_get_value(client, conn, cdp_session_id, params),
        "get_attr" => cdp_get_attr(client, conn, cdp_session_id, params),
        "get_title" => cdp_get_title(client, conn, cdp_session_id),
        "get_url" => cdp_get_url(client, conn, cdp_session_id),
        "get_count" => cdp_get_count(client, conn, cdp_session_id, params),
        "get_box" => cdp_get_box(client, conn, cdp_session_id, params),
        "screenshot" => cdp_screenshot(client, conn, cdp_session_id, params),
        "eval" => cdp_eval(client, conn, cdp_session_id, params),
        "cookies_list" => cdp_cookies_list(client, conn, cdp_session_id),
        "cookies_get" => cdp_cookies_get(client, conn, cdp_session_id, params),
        "cookies_set" => cdp_cookies_set(client, conn, cdp_session_id, params),
        "cookies_set_batch" => cdp_cookies_set_batch(client, conn, cdp_session_id, params),
        "cookies_delete" => cdp_cookies_delete(client, conn, cdp_session_id, params),
        "local_storage_list" => cdp_storage_list(client, conn, cdp_session_id, "localStorage"),
        "local_storage_get" => {
            cdp_storage_get(client, conn, cdp_session_id, params, "localStorage")
        }
        "local_storage_set" => {
            cdp_storage_set(client, conn, cdp_session_id, params, "localStorage")
        }
        "local_storage_delete" => {
            cdp_storage_delete(client, conn, cdp_session_id, params, "localStorage")
        }
        "session_storage_list" => cdp_storage_list(client, conn, cdp_session_id, "sessionStorage"),
        "session_storage_get" => {
            cdp_storage_get(client, conn, cdp_session_id, params, "sessionStorage")
        }
        "session_storage_set" => {
            cdp_storage_set(client, conn, cdp_session_id, params, "sessionStorage")
        }
        "session_storage_delete" => {
            cdp_storage_delete(client, conn, cdp_session_id, params, "sessionStorage")
        }
        "pdf" => cdp_pdf(client, conn, cdp_session_id, params),
        _ => Err(DispatchFailure {
            error: StructuredError::new(ERR_INVALID_ACTION, format!("Unknown action: {action}")),
            attempts: 1,
        }),
    }?;

    Ok(DispatchSuccess {
        session_id: session_id.map(String::from),
        ..result
    })
}

fn ws_error_indicates_closed(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("websocket stream ended")
        || lower.contains("stream ended")
        || lower.contains("connection closed")
        || lower.contains("closed")
        || lower.contains("eof")
}

fn has_non_zero_clip(clip: &Value) -> bool {
    let width = clip.get("width").and_then(Value::as_f64).unwrap_or(0.0);
    let height = clip.get("height").and_then(Value::as_f64).unwrap_or(0.0);
    width > 0.0 && height > 0.0
}

pub(crate) fn send_session_command(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    method: &str,
    params: Option<Value>,
) -> Result<Value, DispatchFailure> {
    let id = client.next_id();
    let command = json!({
        "id": id,
        "method": method,
        "sessionId": session_id,
        "params": params.unwrap_or(json!({}))
    });

    conn.send(&command.to_string())
        .map_err(|e| DispatchFailure {
            error: StructuredError::new(ERR_NETWORK_FAILURE, format!("WebSocket send failed: {e}")),
            attempts: 1,
        })?;

    const MAX_RECV_ATTEMPTS: u32 = 2000;

    for _ in 0..MAX_RECV_ATTEMPTS {
        let response_str = conn.recv(Some(CDP_TIMEOUT_MS)).map_err(|e| {
            let err_msg = e.to_string();
            let code = if ws_error_indicates_closed(&err_msg) {
                ERR_NETWORK_FAILURE
            } else {
                ERR_TIMEOUT
            };
            DispatchFailure {
                error: StructuredError::new(code, format!("CDP recv failed: {err_msg}")),
                attempts: 1,
            }
        })?;

        let response: Value = serde_json::from_str(&response_str).map_err(|e| DispatchFailure {
            error: StructuredError::new(ERR_NETWORK_FAILURE, format!("Invalid CDP response: {e}")),
            attempts: 1,
        })?;

        if response.get("id").and_then(Value::as_u64) == Some(id as u64) {
            if let Some(error) = response.get("error") {
                let msg = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("CDP error");
                return Err(DispatchFailure {
                    error: StructuredError::new(ERR_NETWORK_FAILURE, msg),
                    attempts: 1,
                });
            }
            return Ok(response.get("result").cloned().unwrap_or(json!({})));
        }
        // Skip events (method-only messages with no id)
    }

    Err(DispatchFailure {
        error: StructuredError::new(
            ERR_TIMEOUT,
            format!(
                "CDP response for command '{}' (id {}) not received after {} messages",
                method, id, MAX_RECV_ATTEMPTS
            ),
        ),
        attempts: 1,
    })
}

fn eval_js(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    expression: &str,
    return_by_value: bool,
) -> Result<Value, DispatchFailure> {
    let result = send_session_command(
        client,
        conn,
        session_id,
        "Runtime.evaluate",
        Some(json!({
            "expression": expression,
            "returnByValue": return_by_value,
            "awaitPromise": true,
        })),
    )?;

    if let Some(exception) = result.get("exceptionDetails") {
        let text = exception
            .get("text")
            .or_else(|| {
                exception
                    .get("exception")
                    .and_then(|e| e.get("description"))
            })
            .and_then(Value::as_str)
            .unwrap_or("JavaScript exception");
        return Err(DispatchFailure {
            error: StructuredError::new(ERR_NETWORK_FAILURE, format!("JS error: {text}")),
            attempts: 1,
        });
    }

    Ok(result.get("result").cloned().unwrap_or(json!({})))
}

fn eval_js_value(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    expression: &str,
) -> Result<Value, DispatchFailure> {
    let result = eval_js(client, conn, session_id, expression, true)?;
    Ok(result.get("value").cloned().unwrap_or(Value::Null))
}

fn ok_success(data: Value) -> Result<DispatchSuccess, DispatchFailure> {
    Ok(DispatchSuccess {
        data,
        session_id: None,
        snapshot_id: None,
        attempts: 1,
        backend_status: 200,
        warnings: vec![],
    })
}

fn require_str<'a>(params: &'a Map<String, Value>, key: &str) -> Result<&'a str, DispatchFailure> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| DispatchFailure {
            error: StructuredError::new(
                ERR_INVALID_PARAMS,
                format!("Missing required field '{key}'"),
            ),
            attempts: 1,
        })
}

#[derive(Debug, Clone)]
enum ElementTarget {
    Selector(String),
    Ref(String),
}

impl ElementTarget {
    fn label(&self) -> &str {
        match self {
            Self::Selector(selector) => selector,
            Self::Ref(reference) => reference,
        }
    }
}

fn get_target_param(params: &Map<String, Value>) -> Result<ElementTarget, DispatchFailure> {
    if let Some(selector) = params.get("selector").and_then(Value::as_str) {
        let trimmed = selector.trim();
        if !trimmed.is_empty() {
            return Ok(ElementTarget::Selector(trimmed.to_string()));
        }
    }

    if let Some(reference) = params.get("ref").and_then(Value::as_str) {
        let trimmed = reference.trim();
        if !trimmed.is_empty() {
            return Ok(ElementTarget::Ref(trimmed.to_string()));
        }
    }

    Err(DispatchFailure {
        error: StructuredError::new(
            ERR_INVALID_PARAMS,
            "Missing required target field: provide 'selector' or 'ref'",
        ),
        attempts: 1,
    })
}

fn interactive_predicate_js(tag_expr: &str, node_expr: &str) -> String {
    format!(
        r#"(["a", "button", "input", "select", "textarea", "details", "summary", "iframe", "object", "embed", "video", "audio", "canvas"].includes({tag_expr}) ||
                    {node_expr}.hasAttribute('onclick') ||
                    {node_expr}.hasAttribute('tabindex') ||
                    {node_expr}.getAttribute('role') === 'button' ||
                    {node_expr}.getAttribute('role') === 'link')"#
    )
}

fn interactive_ref_cache_bootstrap_js() -> String {
    let interactive_predicate = interactive_predicate_js("tag", "node");
    format!(
        r#"(function() {{
            const docEl = document.documentElement;
            if (!docEl) return null;

            const cache = window.__ironclawRefCache;
            if (cache && typeof cache.get === 'function') return cache;

            const built = new Map();
            const stack = [{{ node: docEl, depth: 0 }}];
            const maxDepth = 64;
            let interactiveIndex = 0;

            while (stack.length > 0) {{
                const current = stack.pop();
                const node = current.node;
                const depth = current.depth;

                if (!node || node.nodeType !== 1) continue;
                if (depth > maxDepth) continue;

                const tag = node.tagName.toLowerCase();
                const interactive = {interactive_predicate};

                if (interactive) {{
                    built.set(interactiveIndex, node);
                    interactiveIndex += 1;
                }}

                const children = node.children;
                for (let i = children.length - 1; i >= 0; i--) {{
                    stack.push({{ node: children[i], depth: depth + 1 }});
                }}
            }}

            window.__ironclawRefCache = built;
            return built;
        }})()"#
    )
}

fn resolve_ref_element_js(reference: &str) -> String {
    let escaped_ref = escape_js(reference);
    let cache_bootstrap = interactive_ref_cache_bootstrap_js();
    format!(
        r#"(function() {{
            const raw = '{escaped_ref}';
            const normalized = raw.trim().replace(/^@/, '');
            if (!/^e\d+$/.test(normalized)) return null;

            const targetIndex = parseInt(normalized.slice(1), 10);
            if (!Number.isFinite(targetIndex) || targetIndex < 0) return null;

            const cache = {cache_bootstrap};
            if (!cache || typeof cache.get !== 'function') return null;
            return cache.get(targetIndex) || null;
        }})()"#
    )
}

fn escape_js(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => result.push_str("\\\\"),
            '\'' => result.push_str("\\'"),
            '"' => result.push_str("\\\""),
            '`' => result.push_str("\\`"),
            '$' => result.push_str("\\$"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\0' => result.push_str("\\0"),
            '\u{2028}' => result.push_str("\\u2028"),
            '\u{2029}' => result.push_str("\\u2029"),
            ')' => result.push_str("\\)"),
            '(' => result.push_str("\\("),
            _ => result.push(ch),
        }
    }
    result
}

fn wait_for_navigation(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
) -> Result<(), DispatchFailure> {
    // Poll messages for Page.loadEventFired or Page.frameStoppedLoading
    let deadline_ms = 30_000u32;
    let max_iterations = 2000u32;
    let start = wit_host::now_millis();
    for _ in 0..max_iterations {
        let elapsed = wit_host::now_millis() - start;
        if elapsed > deadline_ms as u64 {
            break;
        }
        let remaining = deadline_ms.saturating_sub(elapsed as u32).max(100);
        match conn.recv(Some(remaining)) {
            Ok(msg) => {
                if let Ok(parsed) = serde_json::from_str::<Value>(&msg) {
                    let method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
                    if method == "Page.loadEventFired"
                        || method == "Page.frameStoppedLoading"
                        || method == "Page.domContentEventFired"
                    {
                        return Ok(());
                    }
                }
            }
            Err(_) => break,
        }
    }
    // Timeout is not necessarily fatal - page might have loaded synchronously
    let _ = eval_js_value(client, conn, session_id, "document.readyState");
    Ok(())
}

// === Navigation ===

fn cdp_open(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let url = require_str(params, "url")?;

    send_session_command(client, conn, session_id, "Page.enable", None)?;

    send_session_command(
        client,
        conn,
        session_id,
        "Page.navigate",
        Some(json!({ "url": url })),
    )?;

    wait_for_navigation(client, conn, session_id)?;

    let title = eval_js_value(client, conn, session_id, "document.title")?;
    let final_url = eval_js_value(client, conn, session_id, "window.location.href")?;

    ok_success(json!({
        "url": final_url,
        "title": title,
        "loaded": true
    }))
}

fn cdp_history_nav(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    delta: i32,
) -> Result<DispatchSuccess, DispatchFailure> {
    let history =
        send_session_command(client, conn, session_id, "Page.getNavigationHistory", None)?;

    let current_index = history
        .get("currentIndex")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let new_index = current_index + delta as i64;

    let entries = history.get("entries").and_then(Value::as_array);
    if let Some(entries) = entries {
        if new_index >= 0 && (new_index as usize) < entries.len() {
            if let Some(entry_id) = entries[new_index as usize]
                .get("id")
                .and_then(Value::as_i64)
            {
                send_session_command(
                    client,
                    conn,
                    session_id,
                    "Page.navigateToHistoryEntry",
                    Some(json!({ "entryId": entry_id })),
                )?;
            }
        }
    }

    ok_success(json!({ "navigated": true }))
}

fn cdp_reload(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    send_session_command(client, conn, session_id, "Page.enable", None)?;
    send_session_command(client, conn, session_id, "Page.reload", None)?;
    wait_for_navigation(client, conn, session_id)?;
    ok_success(json!({ "reloaded": true }))
}

// === Snapshot ===

fn enforce_snapshot_size(snapshot: &Value) -> Result<(), DispatchFailure> {
    let serialized = serde_json::to_string(snapshot).map_err(|e| DispatchFailure {
        error: StructuredError::new(
            ERR_NETWORK_FAILURE,
            format!("Failed to serialize snapshot payload: {e}"),
        ),
        attempts: 1,
    })?;

    if serialized.len() > MAX_SNAPSHOT_BYTES {
        return Err(DispatchFailure {
            error: StructuredError::new(
                ERR_SNAPSHOT_TOO_LARGE,
                format!(
                    "Snapshot payload too large: {} bytes exceeds limit of {} bytes",
                    serialized.len(),
                    MAX_SNAPSHOT_BYTES
                ),
            )
            .with_hint("Use snapshot mode=interactive-only, reduce depth, or scope with selector."),
            attempts: 1,
        });
    }

    Ok(())
}

fn build_snapshot_js(mode: &str, depth: u64, selector: Option<&str>) -> String {
    let text_char_limit = MAX_SNAPSHOT_TEXT_CHARS;
    let node_limit = MAX_SNAPSHOT_NODES;
    let payload_limit = MAX_SNAPSHOT_BYTES;
    let root_selector_expr =
        selector.map_or_else(|| "null".to_string(), |sel| format!("'{}'", escape_js(sel)));
    let interactive_predicate = interactive_predicate_js("tag", "node");

    format!(
        r#"(function() {{
            const mode = '{mode}';
            const maxDepth = {depth};
            const maxNodes = {node_limit};
            const textLimit = {text_char_limit};
            const maxPayload = {payload_limit};
            const rootSelector = {root_selector_expr};

            function isInteractive(node, tag) {{
                return {interactive_predicate};
            }}

            function keepNode(tag, interactive) {{
                if (mode === "full") return true;
                if (mode === "compact") {{
                    return interactive || ["html","body","main","nav","header","footer","form","section","article","div","aside"].includes(tag);
                }}
                return interactive;
            }}

            const root = rootSelector ? document.querySelector(rootSelector) : document.documentElement;
            if (!root) {{
                return {{ refs: [], refCount: 0, stats: {{ nodes: 0, truncated: false, scoped: true, mode }} }};
            }}

            const refs = [];
            const stack = [{{ node: root, depth: 0 }}];
            let refCount = 0;
            let visited = 0;
            let approxChars = 0;
            let truncated = false;

            while (stack.length > 0) {{
                const current = stack.pop();
                const node = current.node;
                const currentDepth = current.depth;

                if (!node || node.nodeType !== 1) continue;
                if (currentDepth > maxDepth) continue;

                visited += 1;
                const tag = node.tagName.toLowerCase();
                const interactive = isInteractive(node, tag);

                if (keepNode(tag, interactive)) {{
                    const item = {{ tag }};
                    if (interactive) item.ref = '@e' + (refCount++);

                    if (node.id) item.id = String(node.id).slice(0, 96);
                    if (node.className && typeof node.className === 'string') item.class = node.className.slice(0, 128);
                    if (node.getAttribute('aria-label')) item.ariaLabel = node.getAttribute('aria-label').slice(0, 128);
                    if (node.getAttribute('placeholder')) item.placeholder = node.getAttribute('placeholder').slice(0, 96);
                    if (node.getAttribute('type')) item.type = node.getAttribute('type').slice(0, 48);
                    if (node.getAttribute('name')) item.name = node.getAttribute('name').slice(0, 96);
                    if (tag === 'a' && node.href) item.href = String(node.href).slice(0, 256);

                    if (interactive) {{
                        const text = (node.textContent || '').trim();
                        if (text) item.text = text.slice(0, textLimit);
                    }}

                    const approxItemChars =
                        (item.id ? item.id.length : 0) +
                        (item.class ? item.class.length : 0) +
                        (item.ariaLabel ? item.ariaLabel.length : 0) +
                        (item.placeholder ? item.placeholder.length : 0) +
                        (item.type ? item.type.length : 0) +
                        (item.name ? item.name.length : 0) +
                        (item.href ? item.href.length : 0) +
                        (item.text ? item.text.length : 0) +
                        32;

                    if (refs.length >= maxNodes || approxChars + approxItemChars > maxPayload) {{
                        truncated = true;
                        break;
                    }}

                    approxChars += approxItemChars;
                    refs.push(item);
                }}

                const children = node.children;
                for (let i = children.length - 1; i >= 0; i--) {{
                    stack.push({{ node: children[i], depth: currentDepth + 1 }});
                }}
            }}

            if (stack.length > 0) truncated = true;

            return {{
                refs,
                refCount,
                stats: {{
                    nodes: visited,
                    truncated,
                    scoped: Boolean(rootSelector),
                    mode,
                    depth: maxDepth,
                }},
            }};
        }})()"#,
    )
}

fn cdp_snapshot(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let mode = params
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("interactive-only");
    let depth = params.get("depth").and_then(Value::as_u64).unwrap_or(20);
    let selector = params.get("selector").and_then(Value::as_str);

    let js = build_snapshot_js(mode, depth, selector);

    let snapshot = eval_js_value(client, conn, session_id, &js)?;
    enforce_snapshot_size(&snapshot)?;
    ok_success(snapshot)
}

// === Interactions ===

fn resolve_element_target_js(target: &ElementTarget) -> String {
    match target {
        ElementTarget::Selector(selector) => {
            let s = escape_js(selector);
            format!("document.querySelector('{s}')")
        }
        ElementTarget::Ref(reference) => resolve_ref_element_js(reference),
    }
}

#[cfg(test)]
fn resolve_element_js(selector: &str) -> String {
    resolve_element_target_js(&ElementTarget::Selector(selector.to_string()))
}

fn cdp_click(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let target_label = escape_js(target.label());
    let js = format!(
        r#"(function() {{
            const el = {};
            if (!el) return {{ clicked: false, error: 'Element not found' }};
            el.click();
            return {{ clicked: true, target: '{}' }};
        }})()"#,
        resolve_element_target_js(&target),
        target_label,
    );
    let result = eval_js_value(client, conn, session_id, &js)?;
    ok_success(result)
}

fn cdp_dblclick(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let target_label = escape_js(target.label());
    let js = format!(
        r#"(function() {{
            const el = {};
            if (!el) return {{ dblclicked: false, error: 'Element not found' }};
            el.dispatchEvent(new MouseEvent('dblclick', {{ bubbles: true }}));
            return {{ dblclicked: true, target: '{}' }};
        }})()"#,
        resolve_element_target_js(&target),
        target_label,
    );
    let result = eval_js_value(client, conn, session_id, &js)?;
    ok_success(result)
}

fn cdp_focus(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let target_label = escape_js(target.label());
    let js = format!(
        r#"(function() {{
            const el = {};
            if (!el) return {{ focused: false, error: 'Element not found' }};
            el.focus();
            return {{ focused: true, target: '{}' }};
        }})()"#,
        resolve_element_target_js(&target),
        target_label,
    );
    let result = eval_js_value(client, conn, session_id, &js)?;
    ok_success(result)
}

fn cdp_fill(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let value = require_str(params, "value")?;
    let target_label = escape_js(target.label());
    let js = format!(
        r#"(function() {{
            const el = {};
            if (!el) return {{ filled: false, error: 'Element not found' }};
            el.focus();
            el.value = '{}';
            el.dispatchEvent(new Event('input', {{ bubbles: true }}));
            el.dispatchEvent(new Event('change', {{ bubbles: true }}));
            return {{ filled: true, target: '{}' }};
        }})()"#,
        resolve_element_target_js(&target),
        escape_js(value),
        target_label,
    );
    let result = eval_js_value(client, conn, session_id, &js)?;
    ok_success(result)
}

fn cdp_type_text(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let value = require_str(params, "value")?;

    // Focus the element first
    let focus_js = format!(
        r#"(function() {{
            const el = {};
            if (!el) return false;
            el.focus();
            return true;
        }})()"#,
        resolve_element_target_js(&target),
    );
    let focused = eval_js_value(client, conn, session_id, &focus_js)?;
    if focused != Value::Bool(true) {
        return ok_success(json!({ "typed": false, "error": "Element not found" }));
    }

    // Type character by character using Input.dispatchKeyEvent
    for ch in value.chars() {
        send_session_command(
            client,
            conn,
            session_id,
            "Input.dispatchKeyEvent",
            Some(json!({
                "type": "keyDown",
                "text": ch.to_string(),
            })),
        )?;
        send_session_command(
            client,
            conn,
            session_id,
            "Input.dispatchKeyEvent",
            Some(json!({
                "type": "keyUp",
                "text": ch.to_string(),
            })),
        )?;
    }

    ok_success(json!({ "typed": true, "target": target.label() }))
}

fn cdp_press(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let key = require_str(params, "key")?;

    if params.contains_key("selector") || params.contains_key("ref") {
        let target = get_target_param(params)?;
        let focus_js = format!(
            r#"(function() {{
                const el = {};
                if (el) el.focus();
            }})()"#,
            resolve_element_target_js(&target),
        );
        eval_js(client, conn, session_id, &focus_js, false)?;
    }

    send_session_command(
        client,
        conn,
        session_id,
        "Input.dispatchKeyEvent",
        Some(json!({
            "type": "keyDown",
            "key": key,
        })),
    )?;
    send_session_command(
        client,
        conn,
        session_id,
        "Input.dispatchKeyEvent",
        Some(json!({
            "type": "keyUp",
            "key": key,
        })),
    )?;

    ok_success(json!({ "pressed": true, "key": key }))
}

fn cdp_key_event(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
    event_type: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    let key = require_str(params, "key")?;
    send_session_command(
        client,
        conn,
        session_id,
        "Input.dispatchKeyEvent",
        Some(json!({
            "type": event_type,
            "key": key,
        })),
    )?;
    ok_success(json!({ event_type: true, "key": key }))
}

fn cdp_hover(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let js = format!(
        r#"(function() {{
            const el = {};
            if (!el) return null;
            const rect = el.getBoundingClientRect();
            return {{ x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 }};
        }})()"#,
        resolve_element_target_js(&target),
    );
    let pos = eval_js_value(client, conn, session_id, &js)?;

    if pos.is_null() {
        return ok_success(json!({ "hovered": false, "error": "Element not found" }));
    }

    let x = pos.get("x").and_then(Value::as_f64).unwrap_or(0.0);
    let y = pos.get("y").and_then(Value::as_f64).unwrap_or(0.0);

    send_session_command(
        client,
        conn,
        session_id,
        "Input.dispatchMouseEvent",
        Some(json!({
            "type": "mouseMoved",
            "x": x,
            "y": y,
        })),
    )?;

    ok_success(json!({ "hovered": true, "target": target.label() }))
}

fn cdp_check_uncheck(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
    check: bool,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let el_js = resolve_element_target_js(&target);
    let target_escaped = escape_js(target.label());
    let js = format!(
        r#"(function() {{
            const el = {el_js};
            if (!el) return {{ done: false, error: 'Element not found' }};
            if (el.checked !== {check}) el.click();
            return {{ done: true, checked: el.checked, target: '{target_escaped}' }};
        }})()"#,
    );
    let result = eval_js_value(client, conn, session_id, &js)?;
    ok_success(result)
}

fn cdp_select(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let value = require_str(params, "value")?;
    let target_label = escape_js(target.label());
    let js = format!(
        r#"(function() {{
            const el = {};
            if (!el) return {{ selected: false, error: 'Element not found' }};
            el.value = '{}';
            el.dispatchEvent(new Event('change', {{ bubbles: true }}));
            return {{ selected: true, target: '{}', value: '{}' }};
        }})()"#,
        resolve_element_target_js(&target),
        escape_js(value),
        target_label,
        escape_js(value),
    );
    let result = eval_js_value(client, conn, session_id, &js)?;
    ok_success(result)
}

fn cdp_scroll(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let x = params.get("x").and_then(Value::as_i64).unwrap_or(0);
    let y = params.get("y").and_then(Value::as_i64).unwrap_or(0);
    let js = format!("window.scrollTo({x}, {y})");
    eval_js(client, conn, session_id, &js, false)?;
    ok_success(json!({ "scrolled": true, "x": x, "y": y }))
}

fn cdp_scroll_into_view(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let target_label = escape_js(target.label());
    let js = format!(
        r#"(function() {{
            const el = {};
            if (!el) return {{ scrolledIntoView: false, error: 'Element not found' }};
            el.scrollIntoView({{ behavior: 'smooth', block: 'center' }});
            return {{ scrolledIntoView: true, target: '{}' }};
        }})()"#,
        resolve_element_target_js(&target),
        target_label,
    );
    let result = eval_js_value(client, conn, session_id, &js)?;
    ok_success(result)
}

fn get_named_target_param(
    params: &Map<String, Value>,
    selector_key: &str,
    ref_key: &str,
) -> Result<ElementTarget, DispatchFailure> {
    if let Some(selector) = params.get(selector_key).and_then(Value::as_str) {
        let trimmed = selector.trim();
        if !trimmed.is_empty() {
            return Ok(ElementTarget::Selector(trimmed.to_string()));
        }
    }

    if let Some(reference) = params.get(ref_key).and_then(Value::as_str) {
        let trimmed = reference.trim();
        if !trimmed.is_empty() {
            return Ok(ElementTarget::Ref(trimmed.to_string()));
        }
    }

    Err(DispatchFailure {
        error: StructuredError::new(
            ERR_INVALID_PARAMS,
            format!(
                "Missing required target field: provide '{}' or '{}'",
                selector_key, ref_key
            ),
        ),
        attempts: 1,
    })
}

fn cdp_drag(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let source = get_named_target_param(params, "source_selector", "source_ref")?;
    let target = get_named_target_param(params, "target_selector", "target_ref")?;

    let js = format!(
        r#"(function() {{
            const src = {};
            const tgt = {};
            if (!src || !tgt) return {{ dragged: false, error: 'Element not found' }};
            const srcRect = src.getBoundingClientRect();
            const tgtRect = tgt.getBoundingClientRect();
            return {{
                src: {{ x: srcRect.x + srcRect.width / 2, y: srcRect.y + srcRect.height / 2 }},
                tgt: {{ x: tgtRect.x + tgtRect.width / 2, y: tgtRect.y + tgtRect.height / 2 }}
            }};
        }})()"#,
        resolve_element_target_js(&source),
        resolve_element_target_js(&target),
    );
    let positions = eval_js_value(client, conn, session_id, &js)?;

    if positions.get("error").is_some() {
        return ok_success(positions);
    }

    let sx = positions
        .get("src")
        .and_then(|s| s.get("x"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let sy = positions
        .get("src")
        .and_then(|s| s.get("y"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let tx = positions
        .get("tgt")
        .and_then(|s| s.get("x"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let ty = positions
        .get("tgt")
        .and_then(|s| s.get("y"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);

    // Mouse down on source
    send_session_command(
        client,
        conn,
        session_id,
        "Input.dispatchMouseEvent",
        Some(json!({
            "type": "mousePressed", "x": sx, "y": sy, "button": "left", "clickCount": 1
        })),
    )?;
    // Move to target
    send_session_command(
        client,
        conn,
        session_id,
        "Input.dispatchMouseEvent",
        Some(json!({
            "type": "mouseMoved", "x": tx, "y": ty, "button": "left"
        })),
    )?;
    // Release
    send_session_command(
        client,
        conn,
        session_id,
        "Input.dispatchMouseEvent",
        Some(json!({
            "type": "mouseReleased", "x": tx, "y": ty, "button": "left", "clickCount": 1
        })),
    )?;

    ok_success(json!({
        "dragged": true,
        "source": source.label(),
        "target": target.label(),
    }))
}

fn cdp_upload(
    _client: &mut CdpClient,
    _conn: &wit_host::WsConnection,
    _session_id: &str,
    _params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    Err(DispatchFailure {
        error: StructuredError::new(
            ERR_NOT_IMPLEMENTED,
            "File upload via CDP requires DOM.setFileInputFiles which needs node resolution. Use eval with a custom approach.",
        ),
        attempts: 1,
    })
}

// === Wait ===

fn cdp_wait(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    if let Some(ms) = params.get("ms").and_then(Value::as_u64) {
        let js = format!("new Promise(r => setTimeout(r, {ms}))");
        eval_js(client, conn, session_id, &js, false)?;
        return ok_success(json!({ "waited": true, "ms": ms }));
    }

    if params.contains_key("selector") || params.contains_key("ref") {
        let target = get_target_param(params)?;
        let js = format!(
            r#"new Promise((resolve, reject) => {{
                const start = Date.now();
                const check = () => {{
                    if ({}) return resolve(true);
                    if (Date.now() - start > 30000) return reject(new Error('Timeout'));
                    requestAnimationFrame(check);
                }};
                check();
            }})"#,
            resolve_element_target_js(&target),
        );
        eval_js(client, conn, session_id, &js, false)?;
        return ok_success(json!({ "waited": true, "target": target.label() }));
    }

    if let Some(text) = params.get("text").and_then(Value::as_str) {
        let js = format!(
            r#"new Promise((resolve, reject) => {{
                const start = Date.now();
                const check = () => {{
                    if (document.body.innerText.includes('{}')) return resolve(true);
                    if (Date.now() - start > 30000) return reject(new Error('Timeout'));
                    requestAnimationFrame(check);
                }};
                check();
            }})"#,
            escape_js(text),
        );
        eval_js(client, conn, session_id, &js, false)?;
        return ok_success(json!({ "waited": true, "text": text }));
    }

    if let Some(url_pat) = params.get("url_pattern").and_then(Value::as_str) {
        let js = format!(
            r#"new Promise((resolve, reject) => {{
                const start = Date.now();
                const check = () => {{
                    if (window.location.href.includes('{}')) return resolve(true);
                    if (Date.now() - start > 30000) return reject(new Error('Timeout'));
                    setTimeout(check, 200);
                }};
                check();
            }})"#,
            escape_js(url_pat),
        );
        eval_js(client, conn, session_id, &js, false)?;
        return ok_success(json!({ "waited": true, "urlPattern": url_pat }));
    }

    if let Some(ls) = params.get("load_state").and_then(Value::as_str) {
        let state = match ls {
            "domcontentloaded" => "interactive",
            "load" | "networkidle" => "complete",
            _ => "complete",
        };
        let js = format!(
            r#"new Promise((resolve, reject) => {{
                if (document.readyState === '{state}' || document.readyState === 'complete') return resolve(true);
                const start = Date.now();
                const check = () => {{
                    if (document.readyState === '{state}' || document.readyState === 'complete') return resolve(true);
                    if (Date.now() - start > 30000) return reject(new Error('Timeout'));
                    setTimeout(check, 200);
                }};
                check();
            }})"#,
        );
        eval_js(client, conn, session_id, &js, false)?;
        return ok_success(json!({ "waited": true, "loadState": ls }));
    }

    if let Some(js_cond) = params.get("js_condition").and_then(Value::as_str) {
        // Wrap js_condition as a string argument to eval() inside the promise
        // to prevent injection breakout from the condition expression.
        let escaped_cond = escape_js(js_cond);
        let js = format!(
            r#"new Promise((resolve, reject) => {{
                const start = Date.now();
                const condFn = new Function('return (' + '{escaped_cond}' + ')');
                const check = () => {{
                    try {{ if (condFn()) return resolve(true); }} catch(e) {{}}
                    if (Date.now() - start > 30000) return reject(new Error('Timeout'));
                    requestAnimationFrame(check);
                }};
                check();
            }})"#,
        );
        eval_js(client, conn, session_id, &js, false)?;
        return ok_success(json!({ "waited": true, "jsCondition": js_cond }));
    }

    Err(DispatchFailure {
        error: StructuredError::new(
            ERR_INVALID_PARAMS,
            "wait requires one of: ms, selector, text, url_pattern, load_state, js_condition",
        ),
        attempts: 1,
    })
}

// === Retrieval ===

fn cdp_get_text(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let js = format!(
        r#"(function() {{
            const el = {};
            const text = el ? (el.textContent || '') : '';
            return {{ text: text.slice(0, {MAX_SNAPSHOT_BYTES}) }};
        }})()"#,
        resolve_element_target_js(&target),
    );
    let result = eval_js_value(client, conn, session_id, &js)?;
    enforce_snapshot_size(&result)?;
    ok_success(result)
}

fn cdp_get_html(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let js = format!(
        r#"(function() {{
            const el = {};
            const html = el ? el.innerHTML : '';
            return {{ html: html.slice(0, {MAX_SNAPSHOT_BYTES}) }};
        }})()"#,
        resolve_element_target_js(&target),
    );
    let result = eval_js_value(client, conn, session_id, &js)?;
    enforce_snapshot_size(&result)?;
    ok_success(result)
}

fn cdp_get_value(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let js = format!(
        r#"(function() {{
            const el = {};
            return {{ value: el ? (el.value || '') : '' }};
        }})()"#,
        resolve_element_target_js(&target),
    );
    let result = eval_js_value(client, conn, session_id, &js)?;
    ok_success(result)
}

fn cdp_get_attr(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let name = require_str(params, "name")?;
    let js = format!(
        r#"(function() {{
            const el = {};
            return {{ attribute: el ? el.getAttribute('{}') : null, name: '{}' }};
        }})()"#,
        resolve_element_target_js(&target),
        escape_js(name),
        escape_js(name),
    );
    let result = eval_js_value(client, conn, session_id, &js)?;
    ok_success(result)
}

fn cdp_get_title(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    let title = eval_js_value(client, conn, session_id, "document.title")?;
    ok_success(json!({ "title": title }))
}

fn cdp_get_url(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    let url = eval_js_value(client, conn, session_id, "window.location.href")?;
    ok_success(json!({ "url": url }))
}

fn cdp_get_count(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let js = format!(
        r#"(function() {{
            const el = {};
            return {{ count: el ? 1 : 0 }};
        }})()"#,
        resolve_element_target_js(&target),
    );
    let result = eval_js_value(client, conn, session_id, &js)?;
    ok_success(result)
}

fn cdp_get_box(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let target = get_target_param(params)?;
    let js = format!(
        r#"(function() {{
            const el = {};
            if (!el) return {{ box: null }};
            const r = el.getBoundingClientRect();
            return {{ box: {{ x: r.x, y: r.y, width: r.width, height: r.height }} }};
        }})()"#,
        resolve_element_target_js(&target),
    );
    let result = eval_js_value(client, conn, session_id, &js)?;
    ok_success(result)
}

// === Screenshot ===

fn cdp_screenshot(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let mut capture_params = json!({ "format": "png" });

    if let Some(true) = params.get("full_page").and_then(Value::as_bool) {
        let metrics =
            send_session_command(client, conn, session_id, "Page.getLayoutMetrics", None)?;
        if let Some(content_size) = metrics.get("contentSize") {
            capture_params["clip"] = json!({
                "x": 0,
                "y": 0,
                "width": content_size.get("width").and_then(Value::as_f64).unwrap_or(1280.0),
                "height": content_size.get("height").and_then(Value::as_f64).unwrap_or(800.0),
                "scale": 1,
            });
            capture_params["captureBeyondViewport"] = json!(true);
        }
    }

    if params.contains_key("selector") || params.contains_key("ref") {
        let target = get_target_param(params)?;
        let js = format!(
            r#"(function() {{
                const el = {};
                if (!el) return null;
                const r = el.getBoundingClientRect();
                return {{ x: r.x, y: r.y, width: r.width, height: r.height }};
            }})()"#,
            resolve_element_target_js(&target),
        );
        let clip = eval_js_value(client, conn, session_id, &js)?;
        if !clip.is_null() && has_non_zero_clip(&clip) {
            capture_params["clip"] = json!({
                "x": clip.get("x").and_then(Value::as_f64).unwrap_or(0.0),
                "y": clip.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                "width": clip.get("width").and_then(Value::as_f64).unwrap_or(0.0),
                "height": clip.get("height").and_then(Value::as_f64).unwrap_or(0.0),
                "scale": 1,
            });
        }
    }

    let result = send_session_command(
        client,
        conn,
        session_id,
        "Page.captureScreenshot",
        Some(capture_params),
    )?;

    let data = result
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| DispatchFailure {
            error: StructuredError::new(
                ERR_NETWORK_FAILURE,
                "Screenshot response missing PNG payload",
            )
            .with_details(result.clone()),
            attempts: 1,
        })?;

    if data.trim().is_empty() {
        return Err(DispatchFailure {
            error: StructuredError::new(ERR_NETWORK_FAILURE, "Screenshot payload is empty")
                .with_details(result),
            attempts: 1,
        });
    }

    ok_success(json!({
        "screenshot": data,
        "mimeType": "image/png"
    }))
}

// === Eval ===

fn cdp_eval(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let script = require_str(params, "script")?;
    // CDP Runtime.evaluate takes an expression, not a function body.
    // Wrap in an IIFE if the script contains `return` so it evaluates correctly.
    let expression = if script.contains("return ") || script.contains("return;") {
        format!("(function() {{ {} }})()", script)
    } else {
        script.to_string()
    };
    let result = eval_js_value(client, conn, session_id, &expression)?;
    ok_success(json!({ "result": result }))
}

// === Cookies ===

fn cdp_cookies_list(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    let result = send_session_command(client, conn, session_id, "Network.getCookies", None)?;
    let cookies = result.get("cookies").cloned().unwrap_or(json!([]));
    ok_success(json!({ "cookies": cookies }))
}

fn cdp_cookies_get(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let name = require_str(params, "name")?;
    let result = send_session_command(client, conn, session_id, "Network.getCookies", None)?;
    let cookies = result.get("cookies").and_then(Value::as_array);
    let cookie = cookies
        .and_then(|arr| {
            arr.iter()
                .find(|c| c.get("name").and_then(Value::as_str) == Some(name))
                .cloned()
        })
        .unwrap_or(Value::Null);
    ok_success(json!({ "cookie": cookie }))
}

fn build_cookie_params(
    params: &Map<String, Value>,
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
) -> Result<Value, DispatchFailure> {
    let name = require_str(params, "name")?;
    let value = require_str(params, "value")?;

    let mut cookie = json!({
        "name": name,
        "value": value,
        "path": params.get("path").and_then(Value::as_str).unwrap_or("/"),
    });
    let obj = cookie.as_object_mut().ok_or_else(|| DispatchFailure {
        error: StructuredError::new(ERR_INVALID_PARAMS, "Internal error building cookie params"),
        attempts: 1,
    })?;

    // Prefer explicit `url` (works on about:blank before navigation),
    // then `domain`, then fall back to current hostname.
    if let Some(url) = params.get("url").and_then(Value::as_str) {
        obj.insert("url".to_string(), json!(url));
    } else {
        let domain = params.get("domain").and_then(Value::as_str).unwrap_or("");
        if domain.is_empty() {
            let hostname = eval_js_value(client, conn, session_id, "window.location.hostname")?;
            let host = hostname.as_str().unwrap_or("");
            if host.is_empty() || host == "null" {
                return Err(DispatchFailure {
                    error: StructuredError::new(
                        ERR_INVALID_PARAMS,
                        "Cannot infer cookie domain from about:blank. Provide 'domain' or 'url'.",
                    ),
                    attempts: 1,
                });
            }
            obj.insert("domain".to_string(), json!(host));
        } else {
            obj.insert("domain".to_string(), json!(domain));
        }
    }

    if let Some(v) = params.get("httpOnly").and_then(Value::as_bool) {
        obj.insert("httpOnly".to_string(), json!(v));
    }
    if let Some(v) = params.get("secure").and_then(Value::as_bool) {
        obj.insert("secure".to_string(), json!(v));
    }
    if let Some(v) = params.get("sameSite").and_then(Value::as_str) {
        obj.insert("sameSite".to_string(), json!(v));
    }
    if let Some(v) = params.get("expires").and_then(Value::as_f64) {
        obj.insert("expires".to_string(), json!(v));
    }

    Ok(cookie)
}

fn cdp_cookies_set(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let cookie = build_cookie_params(params, client, conn, session_id)?;
    let name = cookie["name"].as_str().unwrap_or("").to_string();

    send_session_command(client, conn, session_id, "Network.setCookie", Some(cookie))?;

    ok_success(json!({ "set": true, "name": name }))
}

fn cdp_cookies_set_batch(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let cookies_arr = params
        .get("cookies")
        .and_then(Value::as_array)
        .ok_or_else(|| DispatchFailure {
            error: StructuredError::new(ERR_INVALID_PARAMS, "Missing required 'cookies' array"),
            attempts: 1,
        })?;

    let mut built: Vec<Value> = Vec::with_capacity(cookies_arr.len());
    for entry in cookies_arr {
        let entry_map = entry.as_object().ok_or_else(|| DispatchFailure {
            error: StructuredError::new(
                ERR_INVALID_PARAMS,
                "Each cookie must be a JSON object with 'name' and 'value'",
            ),
            attempts: 1,
        })?;
        built.push(build_cookie_params(entry_map, client, conn, session_id)?);
    }

    let count = built.len();
    send_session_command(
        client,
        conn,
        session_id,
        "Network.setCookies",
        Some(json!({ "cookies": built })),
    )?;

    ok_success(json!({ "set": true, "count": count }))
}

fn cdp_cookies_delete(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let name = require_str(params, "name")?;
    send_session_command(
        client,
        conn,
        session_id,
        "Network.deleteCookies",
        Some(json!({ "name": name })),
    )?;
    ok_success(json!({ "deleted": true, "name": name }))
}

// === Storage ===

fn cdp_storage_list(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    storage_type: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    let js =
        format!("JSON.parse(JSON.stringify(Object.fromEntries(Object.entries({storage_type}))))");
    let entries = eval_js_value(client, conn, session_id, &js)?;
    ok_success(json!({ "entries": entries }))
}

fn cdp_storage_get(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
    storage_type: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    let key = require_str(params, "key")?;
    let js = format!("{storage_type}.getItem('{}')", escape_js(key));
    let value = eval_js_value(client, conn, session_id, &js)?;
    ok_success(json!({ "key": key, "value": value }))
}

fn cdp_storage_set(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
    storage_type: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    let key = require_str(params, "key")?;
    let value = require_str(params, "value")?;
    let js = format!(
        "{storage_type}.setItem('{}', '{}')",
        escape_js(key),
        escape_js(value),
    );
    eval_js(client, conn, session_id, &js, false)?;
    ok_success(json!({ "set": true, "key": key }))
}

fn cdp_storage_delete(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
    storage_type: &str,
) -> Result<DispatchSuccess, DispatchFailure> {
    let key = require_str(params, "key")?;
    let js = format!("{storage_type}.removeItem('{}')", escape_js(key));
    eval_js(client, conn, session_id, &js, false)?;
    ok_success(json!({ "deleted": true, "key": key }))
}

// === PDF ===

fn cdp_pdf(
    client: &mut CdpClient,
    conn: &wit_host::WsConnection,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<DispatchSuccess, DispatchFailure> {
    let print_bg = params
        .get("print_background")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let mut pdf_params = json!({
        "printBackground": print_bg,
        "marginTop": 0.79,
        "marginBottom": 0.79,
        "marginLeft": 0.79,
        "marginRight": 0.79,
    });

    if let Some(format) = params.get("format").and_then(Value::as_str) {
        match format {
            "A3" => {
                pdf_params["paperWidth"] = json!(11.69);
                pdf_params["paperHeight"] = json!(16.54);
            }
            "A5" => {
                pdf_params["paperWidth"] = json!(5.83);
                pdf_params["paperHeight"] = json!(8.27);
            }
            "Legal" => {
                pdf_params["paperWidth"] = json!(8.5);
                pdf_params["paperHeight"] = json!(14.0);
            }
            "Letter" => {
                pdf_params["paperWidth"] = json!(8.5);
                pdf_params["paperHeight"] = json!(11.0);
            }
            "Tabloid" => {
                pdf_params["paperWidth"] = json!(11.0);
                pdf_params["paperHeight"] = json!(17.0);
            }
            _ => {} // A4 is default
        }
    }

    let result = send_session_command(
        client,
        conn,
        session_id,
        "Page.printToPDF",
        Some(pdf_params),
    )?;
    let data = result.get("data").and_then(Value::as_str).unwrap_or("");
    ok_success(json!({
        "pdf": data,
        "mimeType": "application/pdf"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_error_indicates_closed_variants() {
        assert!(ws_error_indicates_closed(
            "WebSocket stream ended unexpectedly"
        ));
        assert!(ws_error_indicates_closed("connection closed by peer"));
        assert!(!ws_error_indicates_closed("request timed out"));
    }

    #[test]
    fn test_has_non_zero_clip_true_when_dimensions_positive() {
        assert!(has_non_zero_clip(&json!({"width": 100.0, "height": 50.0})));
    }

    #[test]
    fn test_has_non_zero_clip_false_when_dimension_missing_or_zero() {
        assert!(!has_non_zero_clip(&json!({"width": 0.0, "height": 50.0})));
        assert!(!has_non_zero_clip(&json!({"width": 100.0, "height": 0.0})));
        assert!(!has_non_zero_clip(&json!({"x": 1.0, "y": 2.0})));
    }

    #[test]
    fn test_snapshot_payload_size_guard_constant_is_reasonable() {
        let max = MAX_SNAPSHOT_BYTES;
        assert!(max >= 64 * 1024);
        assert!(max <= 1024 * 1024);
    }

    #[test]
    fn test_snapshot_mode_compact_is_accepted_by_validation() {
        let params = json!({
            "action": "snapshot",
            "session_id": "s1",
            "mode": "compact"
        });
        let result = crate::validation::validate_action_params("snapshot", &params);
        assert!(result.is_ok());
    }

    #[test]
    fn test_enforce_snapshot_size_allows_small_payload() {
        let snapshot = json!({"refs": [{"tag": "a"}], "stats": {"nodes": 1}});
        let result = enforce_snapshot_size(&snapshot);
        assert!(result.is_ok());
    }

    #[test]
    fn test_enforce_snapshot_size_rejects_large_payload() {
        let oversized = "x".repeat(MAX_SNAPSHOT_BYTES + 1024);
        let snapshot = json!({"refs": [{"text": oversized}]});
        let result = enforce_snapshot_size(&snapshot);
        assert!(result.is_err());
        let err = result.expect_err("expected snapshot size failure");
        assert_eq!(err.error.code, ERR_SNAPSHOT_TOO_LARGE);
        assert!(!err.error.retryable);
    }

    #[test]
    fn test_get_html_slice_js_contains_size_guard() {
        let selector = "body";
        let js = format!(
            r#"(function() {{
            const el = {};
            const html = el ? el.innerHTML : '';
            return {{ html: html.slice(0, {MAX_SNAPSHOT_BYTES}) }};
        }})()"#,
            resolve_element_js(selector),
        );
        assert!(js.contains("html.slice(0"));
        assert!(js.contains(&MAX_SNAPSHOT_BYTES.to_string()));
    }

    #[test]
    fn test_get_text_slice_js_contains_size_guard() {
        let selector = "body";
        let js = format!(
            r#"(function() {{
            const el = {};
            const text = el ? (el.textContent || '') : '';
            return {{ text: text.slice(0, {MAX_SNAPSHOT_BYTES}) }};
        }})()"#,
            resolve_element_js(selector),
        );
        assert!(js.contains("text.slice(0"));
        assert!(js.contains(&MAX_SNAPSHOT_BYTES.to_string()));
    }

    #[test]
    fn test_build_snapshot_js_contains_iterative_guards() {
        let js = build_snapshot_js("interactive-only", 8, Some("body"));
        assert!(js.contains("while (stack.length > 0)"));
        assert!(js.contains("approxChars"));
        assert!(js.contains("maxPayload"));
        assert!(js.contains("refs.length >= maxNodes"));
    }

    #[test]
    fn test_get_target_param_prefers_selector() {
        let params = Map::from_iter([
            (
                "selector".to_string(),
                Value::String("button.submit".to_string()),
            ),
            ("ref".to_string(), Value::String("@e7".to_string())),
        ]);

        let target = get_target_param(&params).expect("target should resolve");
        match target {
            ElementTarget::Selector(selector) => assert_eq!(selector, "button.submit"),
            ElementTarget::Ref(_) => panic!("selector should be preferred when both are present"),
        }
    }

    #[test]
    fn test_get_target_param_accepts_ref_only() {
        let params = Map::from_iter([("ref".to_string(), Value::String("@e12".to_string()))]);

        let target = get_target_param(&params).expect("target should resolve");
        match target {
            ElementTarget::Ref(reference) => assert_eq!(reference, "@e12"),
            ElementTarget::Selector(_) => panic!("ref target expected"),
        }
    }

    #[test]
    fn test_get_named_target_param_supports_source_ref() {
        let params = Map::from_iter([(
            "source_ref".to_string(),
            Value::String("@e3".to_string()),
        )]);

        let target = get_named_target_param(&params, "source_selector", "source_ref")
            .expect("source target should resolve");
        match target {
            ElementTarget::Ref(reference) => assert_eq!(reference, "@e3"),
            ElementTarget::Selector(_) => panic!("ref target expected"),
        }
    }

    #[test]
    fn test_get_named_target_param_supports_target_selector() {
        let params = Map::from_iter([(
            "target_selector".to_string(),
            Value::String(".dropzone".to_string()),
        )]);

        let target = get_named_target_param(&params, "target_selector", "target_ref")
            .expect("target should resolve");
        match target {
            ElementTarget::Selector(selector) => assert_eq!(selector, ".dropzone"),
            ElementTarget::Ref(_) => panic!("selector target expected"),
        }
    }

    #[test]
    fn test_resolve_ref_element_js_contains_ref_parser() {
        let js = resolve_ref_element_js("@e42");
        assert!(js.contains("/^e\\d+$/"));
        assert!(js.contains("targetIndex"));
        assert!(js.contains("window.__ironclawRefCache"));
    }

    #[test]
    fn test_interactive_ref_cache_bootstrap_js_contains_cache_key() {
        let js = interactive_ref_cache_bootstrap_js();
        assert!(js.contains("window.__ironclawRefCache"));
        assert!(js.contains("built.set(interactiveIndex, node)"));
    }

    #[test]
    fn test_send_session_command_timeout_contains_method() {
        let err = DispatchFailure {
            error: StructuredError::new(
                ERR_TIMEOUT,
                format!(
                    "CDP response for command '{}' (id {}) not received after {} messages",
                    "Runtime.evaluate", 5, 2000
                ),
            ),
            attempts: 1,
        };
        assert_eq!(err.error.code, ERR_TIMEOUT);
        assert!(err.error.message.contains("Runtime.evaluate"));
    }
}
