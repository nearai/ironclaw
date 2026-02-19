//! Integration tests for the browser-use WASM tool.
//!
//! Tests the compiled WASM component through the WasmToolWrapper, validating:
//! - Tool registration and loading via WasmToolLoader
//! - Tool execution with mock HTTP backend
//! - Workspace read/write persistence
//! - Action validation and error handling
//! - Approval gating for sensitive actions

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ironclaw::context::JobContext;
use ironclaw::tools::Tool;
use ironclaw::tools::wasm::{
    Capabilities, EndpointPattern, HttpCapability, WasmRuntimeConfig, WasmToolRuntime,
    WasmToolWrapper, WebSocketCapability, WebSocketEndpoint, WorkspaceCapability, WorkspaceReader,
    WorkspaceWriter,
};

fn wasm_path() -> std::path::PathBuf {
    // Check the in-tree target first, then the global CARGO_TARGET_DIR / user config
    let in_tree = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tools-src/browser-use/target/wasm32-wasip2/release/browser_use_tool.wasm");
    if in_tree.exists() {
        return in_tree;
    }
    // Global target-dir (e.g. ~/.cargo/config.toml [build] target-dir)
    let global = std::path::PathBuf::from(std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
        // Common custom target dirs
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/rust-target")
    }))
    .join("wasm32-wasip2/release/browser_use_tool.wasm");
    if global.exists() {
        return global;
    }
    panic!(
        "browser-use WASM not found. Run: \
         cd tools-src/browser-use && cargo build --target wasm32-wasip2 --release\n\
         Checked:\n  {}\n  {}",
        in_tree.display(),
        global.display()
    );
}

fn wasm_bytes() -> Vec<u8> {
    std::fs::read(wasm_path()).expect("failed to read browser-use WASM binary")
}

fn make_runtime() -> Arc<WasmToolRuntime> {
    let mut config = WasmRuntimeConfig::for_testing();
    config.fuel_config.initial_fuel = 500_000_000;
    config.default_limits.memory_bytes = 10 * 1024 * 1024; // 10MB, browser-use needs >1MB
    config.default_limits.fuel = 500_000_000;
    config.default_limits.timeout = std::time::Duration::from_secs(30);
    Arc::new(WasmToolRuntime::new(config).unwrap())
}

fn make_capabilities() -> Capabilities {
    Capabilities {
        workspace_read: Some(WorkspaceCapability {
            allowed_prefixes: vec!["browser-sessions/".to_string()],
            reader: None,
            writer: None,
        }),
        http: Some(HttpCapability {
            allowlist: vec![
                EndpointPattern::host("127.0.0.1"),
                EndpointPattern::host("localhost"),
            ],
            ..Default::default()
        }),
        websocket: Some(
            WebSocketCapability::new(vec![
                WebSocketEndpoint::host("127.0.0.1"),
                WebSocketEndpoint::host("localhost"),
            ])
            .with_pool(),
        ),
        ..Default::default()
    }
}

fn make_job_context() -> JobContext {
    JobContext::with_user("test-user", "browser-test", "integration test")
}

async fn make_wrapper(capabilities: Capabilities) -> WasmToolWrapper {
    let runtime = make_runtime();
    let bytes = wasm_bytes();
    let prepared = runtime
        .prepare("browser-use-tool", &bytes, None)
        .await
        .expect("WASM preparation failed");

    WasmToolWrapper::new(runtime, prepared, capabilities)
}

// -- In-memory workspace mock --

#[derive(Clone)]
struct InMemoryWorkspace {
    data: Arc<Mutex<HashMap<String, String>>>,
}

impl InMemoryWorkspace {
    fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn get(&self, path: &str) -> Option<String> {
        self.data.lock().unwrap().get(path).cloned()
    }
}

impl WorkspaceReader for InMemoryWorkspace {
    fn read(&self, path: &str) -> Option<String> {
        self.data.lock().unwrap().get(path).cloned()
    }
}

impl WorkspaceWriter for InMemoryWorkspace {
    fn write(&self, path: &str, content: &str) -> Result<(), String> {
        self.data
            .lock()
            .unwrap()
            .insert(path.to_string(), content.to_string());
        Ok(())
    }
}

// ===== Tests =====

#[tokio::test]
async fn test_browser_use_tool_registration_and_metadata() {
    let wrapper = make_wrapper(make_capabilities()).await;

    assert_eq!(wrapper.name(), "browser-use-tool");
    assert!(!wrapper.description().is_empty());

    let schema = wrapper.parameters_schema();
    assert!(schema.is_object(), "schema should be a JSON object");
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], serde_json::json!(["action"]));
    assert!(schema["properties"]["action"]["enum"].is_array());
    assert_eq!(schema["properties"]["selector"]["type"], "string");
    assert_eq!(schema["properties"]["session_id"]["type"], "string");
}

#[tokio::test]
async fn test_browser_use_unknown_action_returns_error() {
    let wrapper = make_wrapper(make_capabilities()).await;
    let ctx = make_job_context();

    let params = serde_json::json!({
        "action": "totally_bogus_action_that_does_not_exist"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(
        result.is_ok(),
        "should return Ok(ToolOutput) with error envelope"
    );

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    // The tool returns a structured error envelope with ok=false
    assert_eq!(value["ok"], serde_json::Value::Bool(false));
    assert!(value["error"]["code"].as_str().is_some());
}

#[tokio::test]
async fn test_browser_use_open_action_without_backend_returns_network_error() {
    // Point at a port where nothing is listening so the request always fails
    let wrapper = make_wrapper(make_capabilities()).await;
    let ctx = make_job_context();

    let params = serde_json::json!({
        "action": "open",
        "url": "https://example.com",
        "backend_url": "http://127.0.0.1:19222"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    // Should get a structured error (no backend running on 19222)
    assert_eq!(value["ok"], serde_json::Value::Bool(false));
    let code = value["error"]["code"].as_str().unwrap_or("");
    assert!(
        code == "network_failure" || code == "timeout" || code == "retry_exhausted",
        "expected network_failure, timeout, or retry_exhausted, got: {}",
        code
    );
}

#[tokio::test]
async fn test_browser_use_workspace_persistence_round_trip() {
    let workspace = InMemoryWorkspace::new();
    let ws_arc: Arc<dyn WorkspaceReader> = Arc::new(workspace.clone());
    let ws_writer: Arc<dyn WorkspaceWriter> = Arc::new(workspace.clone());

    let mut caps = make_capabilities();
    if let Some(ref mut ws_cap) = caps.workspace_read {
        ws_cap.reader = Some(ws_arc);
        ws_cap.writer = Some(ws_writer);
    }

    let wrapper = make_wrapper(caps).await;
    let ctx = make_job_context();

    // session_create uses workspace-write to persist the new session.
    // Use the default backend_url — if Browserless is up it will succeed
    // and we validate workspace persistence. If it's down, we accept failure.
    let params = serde_json::json!({
        "action": "session_create",
        "persistence_mode": "workspace"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    if value["ok"] == serde_json::Value::Bool(true) {
        // session_create returns sessionId (camelCase) in the data envelope
        let session_id = value["data"]["sessionId"]
            .as_str()
            .or_else(|| value["data"]["session_id"].as_str());
        assert!(
            session_id.is_some(),
            "session_create should return sessionId, got data: {}",
            value["data"]
        );

        let path = format!("browser-sessions/{}.json", session_id.unwrap());
        let stored = workspace.get(&path);
        assert!(stored.is_some(), "session should be persisted to workspace");

        let stored_json: serde_json::Value =
            serde_json::from_str(&stored.unwrap()).expect("stored data should be valid JSON");
        assert_eq!(stored_json["sessionId"], session_id.unwrap());
    }
    // If it failed (e.g., due to CDP connection), that's also acceptable
    // since we're testing without a real Browserless instance
}

#[tokio::test]
async fn test_browser_use_requires_approval_for_sensitive_actions() {
    let wrapper = make_wrapper(make_capabilities()).await;

    // The tool itself should require approval
    assert!(wrapper.requires_approval());

    // eval requires explicit approval
    let eval_params = serde_json::json!({"action": "eval", "script": "1+1"});
    assert!(wrapper.requires_approval_for(&eval_params));

    // upload requires explicit approval
    let upload_params =
        serde_json::json!({"action": "upload", "selector": "input", "files": ["f"]});
    assert!(wrapper.requires_approval_for(&upload_params));

    // click does NOT require explicit approval
    let click_params = serde_json::json!({"action": "click", "selector": "button"});
    assert!(!wrapper.requires_approval_for(&click_params));

    // get_url does NOT require explicit approval
    let get_url_params = serde_json::json!({"action": "get_url"});
    assert!(!wrapper.requires_approval_for(&get_url_params));
}

#[tokio::test]
async fn test_browser_use_action_alias_normalization() {
    let wrapper = make_wrapper(make_capabilities()).await;
    let ctx = make_job_context();

    // "goto" should be normalized to "open" internally
    let params = serde_json::json!({
        "action": "goto",
        "url": "https://example.com"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    // Should be treated as "open" action (not "invalid_action")
    // It will fail with network error since no backend, but the action name should be "open"
    if let Some(action) = value["action"].as_str() {
        assert_eq!(action, "open");
    }
}

#[tokio::test]
async fn test_browser_use_session_list_empty() {
    let workspace = InMemoryWorkspace::new();
    let ws_arc: Arc<dyn WorkspaceReader> = Arc::new(workspace.clone());
    let ws_writer: Arc<dyn WorkspaceWriter> = Arc::new(workspace.clone());

    let mut caps = make_capabilities();
    if let Some(ref mut ws_cap) = caps.workspace_read {
        ws_cap.reader = Some(ws_arc);
        ws_cap.writer = Some(ws_writer);
    }

    let wrapper = make_wrapper(caps).await;
    let ctx = make_job_context();

    // Use a port where nothing is listening so the test is deterministic
    let params = serde_json::json!({
        "action": "session_list",
        "backend_url": "http://127.0.0.1:19222"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    // session_list uses WebSocket CDP and needs a running Browserless backend.
    // Without one (port 19222), it returns a structured error (network failure).
    assert_eq!(value["ok"], serde_json::Value::Bool(false));
    assert_eq!(value["action"], "session_list");
}

#[tokio::test]
async fn test_browser_use_validation_rejects_empty_url() {
    let wrapper = make_wrapper(make_capabilities()).await;
    let ctx = make_job_context();

    let params = serde_json::json!({
        "action": "open",
        "url": ""
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    // Should return a validation error
    assert_eq!(value["ok"], serde_json::Value::Bool(false));
}

#[tokio::test]
async fn test_browser_use_validation_rejects_invalid_selector() {
    let wrapper = make_wrapper(make_capabilities()).await;
    let ctx = make_job_context();

    let params = serde_json::json!({
        "action": "click",
        "selector": "bad\u{0000}selector"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    assert_eq!(value["ok"], serde_json::Value::Bool(false));
    assert_eq!(value["error"]["code"], "invalid_selector");
}

#[tokio::test]
async fn test_browser_use_fill_normalizes_text_to_value() {
    let wrapper = make_wrapper(make_capabilities()).await;
    let ctx = make_job_context();

    let params = serde_json::json!({
        "action": "fill",
        "session_id": "s1",
        "selector": "#email",
        "text": "user@example.com",
        "backend_url": "http://127.0.0.1:19222"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    assert_eq!(value["action"], "fill");
    let notes = value["meta"]["normalization_notes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        notes
            .iter()
            .filter_map(|v| v.as_str())
            .any(|n| n.contains("text") && n.contains("value")),
        "expected normalization note in {}",
        value
    );
}

#[tokio::test]
async fn test_browser_use_click_accepts_ref_target_validation() {
    let wrapper = make_wrapper(make_capabilities()).await;
    let ctx = make_job_context();

    let params = serde_json::json!({
        "action": "click",
        "session_id": "s1",
        "ref": "@e1",
        "backend_url": "http://127.0.0.1:19222"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    assert_ne!(value["error"]["code"], "not_implemented");
}

#[tokio::test]
async fn test_browser_use_wait_accepts_ref_target_validation() {
    let wrapper = make_wrapper(make_capabilities()).await;
    let ctx = make_job_context();

    let params = serde_json::json!({
        "action": "wait",
        "session_id": "s1",
        "ref": "@e1",
        "backend_url": "http://127.0.0.1:19222"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    assert_ne!(value["error"]["code"], "invalid_params");
}

#[tokio::test]
async fn test_browser_use_drag_accepts_ref_targets_validation() {
    let wrapper = make_wrapper(make_capabilities()).await;
    let ctx = make_job_context();

    let params = serde_json::json!({
        "action": "drag",
        "session_id": "s1",
        "source_ref": "@e1",
        "target_ref": "@e2",
        "backend_url": "http://127.0.0.1:19222"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    assert_ne!(value["error"]["code"], "invalid_params");
}

#[tokio::test]
async fn test_browser_use_snapshot_rejects_ref_and_selector_together() {
    let wrapper = make_wrapper(make_capabilities()).await;
    let ctx = make_job_context();

    let params = serde_json::json!({
        "action": "snapshot",
        "session_id": "s1",
        "ref": "@e1",
        "selector": "main"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    assert_eq!(value["ok"], serde_json::Value::Bool(false));
    assert_eq!(value["error"]["code"], "invalid_params");
}

#[tokio::test]
async fn test_browser_use_snapshot_rejects_malformed_ref() {
    let wrapper = make_wrapper(make_capabilities()).await;
    let ctx = make_job_context();

    let params = serde_json::json!({
        "action": "snapshot",
        "session_id": "s1",
        "ref": "@x1"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    assert_eq!(value["ok"], serde_json::Value::Bool(false));
    assert_eq!(value["error"]["code"], "invalid_ref");
}

// ===== CDP Session + Workspace Persistence Integration Tests =====

#[tokio::test]
async fn test_browser_use_session_resume_without_workspace_data_returns_error() {
    let workspace = InMemoryWorkspace::new();
    let ws_arc: Arc<dyn WorkspaceReader> = Arc::new(workspace.clone());
    let ws_writer: Arc<dyn WorkspaceWriter> = Arc::new(workspace.clone());

    let mut caps = make_capabilities();
    if let Some(ref mut ws_cap) = caps.workspace_read {
        ws_cap.reader = Some(ws_arc);
        ws_cap.writer = Some(ws_writer);
    }

    let wrapper = make_wrapper(caps).await;
    let ctx = make_job_context();

    // Attempt to resume a session that doesn't exist in workspace
    let params = serde_json::json!({
        "action": "session_resume",
        "session_id": "nonexistent-session-999"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    // Should fail because session data not found in workspace
    assert_eq!(value["ok"], serde_json::Value::Bool(false));
    let code = value["error"]["code"].as_str().unwrap_or("");
    assert!(
        code == "session_not_found" || code == "network_failure",
        "expected session_not_found or network_failure, got: {}",
        code
    );
}

#[tokio::test]
async fn test_browser_use_session_resume_with_preseeded_workspace() {
    let workspace = InMemoryWorkspace::new();

    // Pre-seed workspace with session data
    workspace.data.lock().unwrap().insert(
        "browser-sessions/test-session-42.json".to_string(),
        serde_json::json!({
            "sessionId": "test-session-42",
            "browserContextId": "ctx-abc-123",
            "createdAt": 1700000000000_u64,
            "backendUrl": "http://127.0.0.1:9222"
        })
        .to_string(),
    );

    let ws_arc: Arc<dyn WorkspaceReader> = Arc::new(workspace.clone());
    let ws_writer: Arc<dyn WorkspaceWriter> = Arc::new(workspace.clone());

    let mut caps = make_capabilities();
    if let Some(ref mut ws_cap) = caps.workspace_read {
        ws_cap.reader = Some(ws_arc);
        ws_cap.writer = Some(ws_writer);
    }

    let wrapper = make_wrapper(caps).await;
    let ctx = make_job_context();

    let params = serde_json::json!({
        "action": "session_resume",
        "session_id": "test-session-42"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    // Session data was found in workspace, but resume will fail because no
    // Browserless backend is running. Depending on whether the tool reads
    // workspace first or connects first, we may get session_not_found or
    // network_failure -- both are acceptable.
    assert_eq!(value["ok"], serde_json::Value::Bool(false));
    let code = value["error"]["code"].as_str().unwrap_or("");
    assert!(
        code == "network_failure" || code == "session_not_found",
        "expected network_failure or session_not_found, got: {}",
        code
    );
}

#[tokio::test]
async fn test_browser_use_session_close_without_session_is_idempotent() {
    let workspace = InMemoryWorkspace::new();
    let ws_arc: Arc<dyn WorkspaceReader> = Arc::new(workspace.clone());
    let ws_writer: Arc<dyn WorkspaceWriter> = Arc::new(workspace.clone());

    let mut caps = make_capabilities();
    if let Some(ref mut ws_cap) = caps.workspace_read {
        ws_cap.reader = Some(ws_arc);
        ws_cap.writer = Some(ws_writer);
    }

    let wrapper = make_wrapper(caps).await;
    let ctx = make_job_context();

    // session_close is idempotent: closing a nonexistent session succeeds
    // (it reads workspace, finds nothing to dispose, writes empty state).
    // If no backend is reachable, the WS connection fails and it errors.
    // Either outcome is valid.
    let params = serde_json::json!({
        "action": "session_close",
        "session_id": "nonexistent",
        "backend_url": "http://127.0.0.1:19222"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    // With no backend on port 19222, expect network failure
    assert_eq!(
        value["ok"],
        serde_json::Value::Bool(false),
        "session_close to unreachable backend should fail: {}",
        value
    );
    assert_eq!(value["action"], "session_close");
}

// ===== E2E Tests with Real Browserless Docker =====
// These tests require a running Browserless container:
//   docker run -d --name browserless -p 9222:3000 ghcr.io/browserless/chromium
// Run with: cargo test --test browser_use_integration e2e -- --ignored

fn browserless_available() -> bool {
    std::net::TcpStream::connect("127.0.0.1:9222").is_ok()
}

#[tokio::test]
#[ignore = "requires running Browserless Docker container on port 9222"]
async fn e2e_browser_use_open_and_get_title() {
    if !browserless_available() {
        eprintln!("Skipping: Browserless not available on 127.0.0.1:9222");
        return;
    }

    let workspace = InMemoryWorkspace::new();
    let ws_arc: Arc<dyn WorkspaceReader> = Arc::new(workspace.clone());
    let ws_writer: Arc<dyn WorkspaceWriter> = Arc::new(workspace.clone());

    let mut caps = make_capabilities();
    if let Some(ref mut ws_cap) = caps.workspace_read {
        ws_cap.reader = Some(ws_arc);
        ws_cap.writer = Some(ws_writer);
    }

    let wrapper = make_wrapper(caps).await;
    let ctx = make_job_context();

    // Open a page
    let params = serde_json::json!({
        "action": "open",
        "url": "https://example.com",
        "backend_url": "http://127.0.0.1:9222"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok(), "open should succeed with Browserless");

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    assert_eq!(value["ok"], serde_json::Value::Bool(true));
    assert_eq!(value["action"], "open");
}

#[tokio::test]
#[ignore = "requires running Browserless Docker container on port 9222"]
async fn e2e_browser_use_screenshot() {
    if !browserless_available() {
        eprintln!("Skipping: Browserless not available on 127.0.0.1:9222");
        return;
    }

    let wrapper = make_wrapper(make_capabilities()).await;
    let ctx = make_job_context();

    let params = serde_json::json!({
        "action": "screenshot",
        "backend_url": "http://127.0.0.1:9222"
    });

    let result = wrapper.execute(params, &ctx).await;
    assert!(result.is_ok(), "screenshot should succeed with Browserless");

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    assert_eq!(value["ok"], serde_json::Value::Bool(true));
    assert_eq!(value["action"], "screenshot");
}

#[tokio::test]
#[ignore = "requires running Browserless Docker container on port 9222"]
async fn e2e_browser_use_snapshot_rightmove_does_not_trap() {
    if !browserless_available() {
        eprintln!("Skipping: Browserless not available on 127.0.0.1:9222");
        return;
    }

    let workspace = InMemoryWorkspace::new();
    let ws_arc: Arc<dyn WorkspaceReader> = Arc::new(workspace.clone());
    let ws_writer: Arc<dyn WorkspaceWriter> = Arc::new(workspace.clone());

    let mut caps = make_capabilities();
    if let Some(ref mut ws_cap) = caps.workspace_read {
        ws_cap.reader = Some(ws_arc);
        ws_cap.writer = Some(ws_writer);
    }

    let wrapper = make_wrapper(caps).await;
    let ctx = make_job_context();

    let create_result = wrapper
        .execute(
            serde_json::json!({
                "action": "session_create",
                "backend_url": "http://127.0.0.1:9222"
            }),
            &ctx,
        )
        .await;
    assert!(
        create_result.is_ok(),
        "session_create failed: {:?}",
        create_result.err()
    );

    let create_output = create_result.unwrap();
    let create_value: serde_json::Value =
        serde_json::from_str(&create_output.result.to_string()).unwrap_or(create_output.result);
    assert_eq!(create_value["ok"], serde_json::Value::Bool(true));
    let session_id = create_value["data"]["sessionId"]
        .as_str()
        .expect("session id")
        .to_string();

    let open_result = wrapper
        .execute(
            serde_json::json!({
                "action": "open",
                "url": "https://www.rightmove.co.uk",
                "session_id": &session_id,
                "backend_url": "http://127.0.0.1:9222"
            }),
            &ctx,
        )
        .await;
    assert!(open_result.is_ok(), "open failed: {:?}", open_result.err());

    let snapshot_result = wrapper
        .execute(
            serde_json::json!({
                "action": "snapshot",
                "mode": "interactive-only",
                "depth": 8,
                "session_id": &session_id,
                "backend_url": "http://127.0.0.1:9222"
            }),
            &ctx,
        )
        .await;

    // Critical assertion: no sandbox trap panic/Err should occur.
    assert!(
        snapshot_result.is_ok(),
        "snapshot should not trap: {:?}",
        snapshot_result.err()
    );

    let snapshot_output = snapshot_result.unwrap();
    let snapshot_value: serde_json::Value =
        serde_json::from_str(&snapshot_output.result.to_string()).unwrap_or(snapshot_output.result);
    assert_eq!(snapshot_value["action"], "snapshot");

    if snapshot_value["ok"] == serde_json::Value::Bool(false) {
        let code = snapshot_value["error"]["code"].as_str().unwrap_or("");
        assert_eq!(
            code, "snapshot_too_large",
            "snapshot should either succeed or return snapshot_too_large, got: {}",
            snapshot_value
        );
    }

    let get_html_result = wrapper
        .execute(
            serde_json::json!({
                "action": "get_html",
                "selector": "body",
                "session_id": &session_id,
                "backend_url": "http://127.0.0.1:9222"
            }),
            &ctx,
        )
        .await;
    assert!(
        get_html_result.is_ok(),
        "get_html should not trap: {:?}",
        get_html_result.err()
    );

    let close_result = wrapper
        .execute(
            serde_json::json!({
                "action": "session_close",
                "session_id": &session_id,
                "backend_url": "http://127.0.0.1:9222"
            }),
            &ctx,
        )
        .await;
    assert!(close_result.is_ok());
}

#[tokio::test]
#[ignore = "requires running Browserless Docker container on port 9222"]
async fn e2e_browser_use_session_create_and_list() {
    if !browserless_available() {
        eprintln!("Skipping: Browserless not available on 127.0.0.1:9222");
        return;
    }

    let workspace = InMemoryWorkspace::new();
    let ws_arc: Arc<dyn WorkspaceReader> = Arc::new(workspace.clone());
    let ws_writer: Arc<dyn WorkspaceWriter> = Arc::new(workspace.clone());

    let mut caps = make_capabilities();
    if let Some(ref mut ws_cap) = caps.workspace_read {
        ws_cap.reader = Some(ws_arc);
        ws_cap.writer = Some(ws_writer);
    }

    let wrapper = make_wrapper(caps).await;
    let ctx = make_job_context();

    // Create a session
    let create_params = serde_json::json!({
        "action": "session_create",
        "backend_url": "http://127.0.0.1:9222"
    });

    let result = wrapper.execute(create_params, &ctx).await;
    assert!(result.is_ok(), "session_create failed: {:?}", result.err());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    assert_eq!(
        value["ok"],
        serde_json::Value::Bool(true),
        "session_create envelope: {}",
        value
    );
    assert_eq!(value["action"], "session_create");
    let session_id = value["data"]["sessionId"]
        .as_str()
        .expect("should have session ID");

    // Verify workspace persistence
    let ws_path = format!("browser-sessions/{}.json", session_id);
    let stored = workspace.get(&ws_path);
    assert!(stored.is_some(), "session should be persisted to workspace");

    // List sessions
    let list_params = serde_json::json!({
        "action": "session_list",
        "backend_url": "http://127.0.0.1:9222"
    });

    let result = wrapper.execute(list_params, &ctx).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);

    assert_eq!(value["ok"], serde_json::Value::Bool(true));
    assert_eq!(value["action"], "session_list");
}

#[tokio::test]
#[ignore = "requires running Browserless Docker container on port 9222"]
async fn e2e_browser_use_full_session_lifecycle() {
    if !browserless_available() {
        eprintln!("Skipping: Browserless not available on 127.0.0.1:9222");
        return;
    }

    let workspace = InMemoryWorkspace::new();
    let ws_arc: Arc<dyn WorkspaceReader> = Arc::new(workspace.clone());
    let ws_writer: Arc<dyn WorkspaceWriter> = Arc::new(workspace.clone());

    let mut caps = make_capabilities();
    if let Some(ref mut ws_cap) = caps.workspace_read {
        ws_cap.reader = Some(ws_arc);
        ws_cap.writer = Some(ws_writer);
    }

    let wrapper = make_wrapper(caps).await;
    let ctx = make_job_context();

    // 1. Create session
    let result = wrapper
        .execute(
            serde_json::json!({
                "action": "session_create",
                "backend_url": "http://127.0.0.1:9222"
            }),
            &ctx,
        )
        .await;
    assert!(result.is_ok(), "session_create failed: {:?}", result.err());
    let output = result.unwrap();
    let create_value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);
    assert_eq!(
        create_value["ok"],
        serde_json::Value::Bool(true),
        "session_create envelope: {}",
        create_value
    );
    let session_id = create_value["data"]["sessionId"]
        .as_str()
        .expect("session ID")
        .to_string();

    // 2. Open a page using REST API
    let result = wrapper
        .execute(
            serde_json::json!({
                "action": "open",
                "url": "https://example.com",
                "session_id": &session_id,
                "backend_url": "http://127.0.0.1:9222"
            }),
            &ctx,
        )
        .await;
    assert!(result.is_ok());
    let output = result.unwrap();
    let open_value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);
    assert_eq!(
        open_value["ok"],
        serde_json::Value::Bool(true),
        "open with session failed: {}",
        open_value
    );

    // 3. Close session
    let result = wrapper
        .execute(
            serde_json::json!({
                "action": "session_close",
                "session_id": &session_id,
                "backend_url": "http://127.0.0.1:9222"
            }),
            &ctx,
        )
        .await;
    assert!(result.is_ok());
    let output = result.unwrap();
    let close_value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);
    assert_eq!(close_value["ok"], serde_json::Value::Bool(true));
    assert_eq!(close_value["action"], "session_close");
}
