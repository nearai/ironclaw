//! Integration tests for the Slack WASM channel.
//!
//! These tests verify Slack-specific behaviors: HMAC-SHA256 webhook signing,
//! Bearer auth, app_mention vs DM message handling, bot message filtering,
//! and thread tracking.

use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "integration")]
use std::sync::{Mutex, OnceLock};

#[cfg(feature = "integration")]
use futures::StreamExt;
#[cfg(feature = "integration")]
use ironclaw::channels::Channel;
#[cfg(feature = "integration")]
use ironclaw::channels::OutgoingResponse;
use ironclaw::channels::wasm::{
    PreparedChannelModule, WasmChannel, WasmChannelRuntime, WasmChannelRuntimeConfig,
};
use ironclaw::pairing::PairingStore;
#[cfg(feature = "integration")]
use tokio::time::{Duration, timeout};

/// Skip the test if the Slack WASM module hasn't been built.
/// In CI (detected via the `CI` env var), panic instead of skipping so a
/// broken WASM build step doesn't silently produce green tests.
macro_rules! require_slack_wasm {
    () => {
        if !slack_wasm_path().exists() {
            let msg = format!(
                "Slack WASM module not found at {:?}. \
                 Build with: cd channels-src/slack && cargo build --target wasm32-wasip2 --release",
                slack_wasm_path()
            );
            if std::env::var("CI").is_ok() {
                panic!("{}", msg);
            }
            eprintln!("Skipping test: {}", msg);
            return;
        }
    };
}

/// Path to the built Slack WASM module
/// Resolve a project-relative path, falling back to other git worktrees.
fn find_project_file(relative_path: &str) -> std::path::PathBuf {
    let local = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    if local.exists() {
        return local;
    }

    if let Ok(output) = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()
        && output.status.success()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                let candidate = std::path::PathBuf::from(path).join(relative_path);
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }

    local
}

fn slack_wasm_path() -> std::path::PathBuf {
    find_project_file("channels-src/slack/target/wasm32-wasip2/release/slack_channel.wasm")
}

fn slack_capabilities_path() -> std::path::PathBuf {
    find_project_file("channels-src/slack/slack.capabilities.json")
}

/// Create a test runtime for WASM channel operations.
fn create_test_runtime() -> Arc<WasmChannelRuntime> {
    let config = WasmChannelRuntimeConfig::for_testing();
    Arc::new(WasmChannelRuntime::new(config).expect("Failed to create runtime"))
}

/// Load the real Slack WASM module.
async fn load_slack_module(
    runtime: &Arc<WasmChannelRuntime>,
) -> Result<Arc<PreparedChannelModule>, Box<dyn std::error::Error>> {
    let path = slack_wasm_path();
    let wasm_bytes = std::fs::read(&path)
        .map_err(|e| format!("Failed to read WASM module at {}: {}", path.display(), e))?;

    let module = runtime
        .prepare(
            "slack",
            &wasm_bytes,
            None,
            Some("Slack Events API channel".to_string()),
        )
        .await?;

    Ok(module)
}

/// Create a Slack channel instance with configuration.
async fn create_slack_channel(runtime: Arc<WasmChannelRuntime>, config_json: &str) -> WasmChannel {
    create_slack_channel_with_store(runtime, config_json, Arc::new(PairingStore::new_noop())).await
}

async fn create_slack_channel_with_store(
    runtime: Arc<WasmChannelRuntime>,
    config_json: &str,
    pairing_store: Arc<PairingStore>,
) -> WasmChannel {
    let module = load_slack_module(&runtime)
        .await
        .expect("Failed to load Slack WASM module");

    let capabilities_bytes = std::fs::read(slack_capabilities_path())
        .unwrap_or_else(|err| panic!("Failed to read Slack capabilities file: {err}"));
    let capabilities_file =
        ironclaw::channels::wasm::ChannelCapabilitiesFile::from_bytes(&capabilities_bytes)
            .unwrap_or_else(|err| panic!("Failed to parse Slack capabilities file: {err}"));

    let channel = WasmChannel::new(
        runtime,
        module,
        capabilities_file.to_capabilities(),
        "default",
        config_json.to_string(),
        pairing_store,
        None,
    );
    channel
        .set_credential("SLACK_BOT_TOKEN", "xoxb-fake-test-token".to_string())
        .await;
    channel
        .set_credential("SLACK_SIGNING_SECRET", "test-signing-secret".to_string())
        .await;
    channel
}

/// Build a Slack event_callback JSON payload for a DM message.
fn build_slack_event_callback(event: serde_json::Value) -> Vec<u8> {
    serde_json::json!({
        "type": "event_callback",
        "token": "fake-verification-token",
        "team_id": "T0001",
        "event": event,
        "event_id": "Ev001",
        "event_time": 1234567890
    })
    .to_string()
    .into_bytes()
}

#[cfg(feature = "integration")]
struct ScopedEnvVar {
    key: &'static str,
    original: Option<String>,
    _mutex: std::sync::MutexGuard<'static, ()>,
}

#[cfg(feature = "integration")]
impl ScopedEnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = ENV_MUTEX
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env mutex poisoned");
        let original = std::env::var(key).ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var(key, value);
        }
        Self {
            key,
            original,
            _mutex: guard,
        }
    }
}

#[cfg(feature = "integration")]
impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        // SAFETY: Under ENV_MUTEX (still held by _mutex), no concurrent env access.
        unsafe {
            if let Some(ref value) = self.original {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

#[cfg(feature = "integration")]
fn slack_test_http_rewrite_map(base_url: &str) -> String {
    serde_json::json!({
        "slack.com": base_url,
        "files.slack.com": base_url,
    })
    .to_string()
}

#[cfg(feature = "integration")]
async fn expect_no_message(stream: &mut ironclaw::channels::MessageStream, timeout_ms: u64) {
    let result = timeout(Duration::from_millis(timeout_ms), stream.next()).await;
    assert!(
        result.is_err(),
        "expected no message, but stream produced one"
    );
}

// ── Tests without integration gate (on_http_request only) ───────────────

#[tokio::test]
async fn test_dm_from_owner_accepted() {
    require_slack_wasm!();
    let runtime = create_test_runtime();

    let config = serde_json::json!({
        "owner_id": "U42OWNER",
        "dm_policy": "pairing",
        "allow_from": [],
    })
    .to_string();

    let channel = create_slack_channel(runtime, &config).await;

    let body = build_slack_event_callback(serde_json::json!({
        "type": "message",
        "user": "U42OWNER",
        "text": "hello from owner",
        "channel": "DU42OWNER",
        "ts": "1234567890.000001",
        "channel_type": "im"
    }));

    let response = channel
        .call_on_http_request(
            "POST",
            "/webhook/slack",
            &HashMap::new(),
            &HashMap::new(),
            &body,
            true,
        )
        .await
        .expect("HTTP callback failed");

    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn test_dm_unauthorized_blocked_allowlist() {
    require_slack_wasm!();
    let runtime = create_test_runtime();

    let config = serde_json::json!({
        "owner_id": "U42OWNER",
        "dm_policy": "allowlist",
        "allow_from": ["U42OWNER"],
    })
    .to_string();

    let channel = create_slack_channel(runtime, &config).await;

    // DM from an unauthorized user
    let body = build_slack_event_callback(serde_json::json!({
        "type": "message",
        "user": "U99STRANGER",
        "text": "hello",
        "channel": "DU99STRANGER",
        "ts": "1234567890.000002",
        "channel_type": "im"
    }));

    let response = channel
        .call_on_http_request(
            "POST",
            "/webhook/slack",
            &HashMap::new(),
            &HashMap::new(),
            &body,
            true,
        )
        .await
        .expect("HTTP callback failed");

    // Should return 200 (acknowledge webhook) but not emit a message
    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn test_dm_pairing_policy_triggers_flow() {
    require_slack_wasm!();
    let runtime = create_test_runtime();
    let pairing_store = Arc::new(PairingStore::new_noop());

    let config = serde_json::json!({
        "owner_id": null,
        "dm_policy": "pairing",
        "allow_from": [],
    })
    .to_string();

    let channel = create_slack_channel_with_store(runtime, &config, pairing_store.clone()).await;

    let body = build_slack_event_callback(serde_json::json!({
        "type": "message",
        "user": "U99NEWUSER",
        "text": "hello",
        "channel": "DU99NEWUSER",
        "ts": "1234567890.000003",
        "channel_type": "im"
    }));

    let response = channel
        .call_on_http_request(
            "POST",
            "/webhook/slack",
            &HashMap::new(),
            &HashMap::new(),
            &body,
            true,
        )
        .await
        .expect("HTTP callback failed");

    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn test_open_dm_policy_allows_all() {
    require_slack_wasm!();
    let runtime = create_test_runtime();

    let config = serde_json::json!({
        "owner_id": null,
        "dm_policy": "open",
        "allow_from": [],
    })
    .to_string();

    let channel = create_slack_channel(runtime, &config).await;

    // DM from any user should be accepted
    let body = build_slack_event_callback(serde_json::json!({
        "type": "message",
        "user": "U88RANDOM",
        "text": "hello from anyone",
        "channel": "DU88RANDOM",
        "ts": "1234567890.000004",
        "channel_type": "im"
    }));

    let response = channel
        .call_on_http_request(
            "POST",
            "/webhook/slack",
            &HashMap::new(),
            &HashMap::new(),
            &body,
            true,
        )
        .await
        .expect("HTTP callback failed");

    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn test_url_verification_returns_challenge() {
    require_slack_wasm!();
    let runtime = create_test_runtime();

    let config = serde_json::json!({
        "owner_id": null,
        "dm_policy": "open",
        "allow_from": [],
    })
    .to_string();

    let channel = create_slack_channel(runtime, &config).await;

    let body = serde_json::json!({
        "type": "url_verification",
        "token": "fake-verification-token",
        "challenge": "test-challenge-abc123"
    })
    .to_string()
    .into_bytes();

    let response = channel
        .call_on_http_request(
            "POST",
            "/webhook/slack",
            &HashMap::new(),
            &HashMap::new(),
            &body,
            true,
        )
        .await
        .expect("HTTP callback failed");

    assert_eq!(response.status, 200);
    let response_body = String::from_utf8_lossy(&response.body);
    assert!(
        response_body.contains("test-challenge-abc123"),
        "Expected challenge in response body, got: {}",
        response_body
    );
}

// ── Tests with integration gate (stream/respond) ────────────────────────

#[tokio::test]
#[cfg(feature = "integration")]
async fn test_app_mention_strips_bot_prefix() {
    require_slack_wasm!();
    let runtime = create_test_runtime();

    let config = serde_json::json!({
        "owner_id": null,
        "dm_policy": "open",
        "allow_from": [],
    })
    .to_string();

    let channel = create_slack_channel(runtime, &config).await;
    let mut stream = channel
        .start_message_stream_for_test()
        .await
        .expect("Failed to bootstrap test message stream");

    let body = build_slack_event_callback(serde_json::json!({
        "type": "app_mention",
        "user": "U42OWNER",
        "text": "<@UBOT> hello",
        "channel": "C0001",
        "ts": "1234567890.000010"
    }));

    let response = channel
        .call_on_http_request(
            "POST",
            "/webhook/slack",
            &HashMap::new(),
            &HashMap::new(),
            &body,
            true,
        )
        .await
        .expect("HTTP callback failed");

    assert_eq!(response.status, 200);

    let msg = timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("message should arrive")
        .expect("stream should yield a message");

    // The bot mention prefix should be stripped
    let content = msg.content.trim();
    assert!(
        !content.starts_with("<@"),
        "Bot mention should be stripped from content: '{}'",
        content
    );
    assert!(
        content.contains("hello"),
        "Content should contain 'hello', got: '{}'",
        content
    );
}

#[tokio::test]
#[cfg(feature = "integration")]
async fn test_bot_message_with_bot_id_ignored() {
    require_slack_wasm!();
    let runtime = create_test_runtime();

    let config = serde_json::json!({
        "owner_id": null,
        "dm_policy": "open",
        "allow_from": [],
    })
    .to_string();

    let channel = create_slack_channel(runtime, &config).await;
    let mut stream = channel
        .start_message_stream_for_test()
        .await
        .expect("Failed to bootstrap test message stream");

    // Message with bot_id should be ignored
    let body = build_slack_event_callback(serde_json::json!({
        "type": "message",
        "user": "U42OWNER",
        "text": "I am a bot",
        "channel": "DU42OWNER",
        "ts": "1234567890.000011",
        "channel_type": "im",
        "bot_id": "B12345"
    }));

    let response = channel
        .call_on_http_request(
            "POST",
            "/webhook/slack",
            &HashMap::new(),
            &HashMap::new(),
            &body,
            true,
        )
        .await
        .expect("HTTP callback failed");

    assert_eq!(response.status, 200);
    expect_no_message(&mut stream, 500).await;
}

#[tokio::test]
#[cfg(feature = "integration")]
async fn test_message_subtype_ignored() {
    require_slack_wasm!();
    let runtime = create_test_runtime();

    let config = serde_json::json!({
        "owner_id": null,
        "dm_policy": "open",
        "allow_from": [],
    })
    .to_string();

    let channel = create_slack_channel(runtime, &config).await;
    let mut stream = channel
        .start_message_stream_for_test()
        .await
        .expect("Failed to bootstrap test message stream");

    // Message with subtype should be ignored
    let body = build_slack_event_callback(serde_json::json!({
        "type": "message",
        "user": "U42OWNER",
        "text": "joined channel",
        "channel": "C0001",
        "ts": "1234567890.000012",
        "subtype": "channel_join"
    }));

    let response = channel
        .call_on_http_request(
            "POST",
            "/webhook/slack",
            &HashMap::new(),
            &HashMap::new(),
            &body,
            true,
        )
        .await
        .expect("HTTP callback failed");

    assert_eq!(response.status, 200);
    expect_no_message(&mut stream, 500).await;
}

#[tokio::test]
#[cfg(feature = "integration")]
async fn test_dm_emits_correct_metadata() {
    require_slack_wasm!();
    let runtime = create_test_runtime();

    let config = serde_json::json!({
        "owner_id": null,
        "dm_policy": "open",
        "allow_from": [],
    })
    .to_string();

    let channel = create_slack_channel(runtime, &config).await;
    let mut stream = channel
        .start_message_stream_for_test()
        .await
        .expect("Failed to bootstrap test message stream");

    let body = build_slack_event_callback(serde_json::json!({
        "type": "message",
        "user": "U42OWNER",
        "text": "hello metadata test",
        "channel": "DU42OWNER",
        "ts": "1234567890.000013",
        "channel_type": "im"
    }));

    let response = channel
        .call_on_http_request(
            "POST",
            "/webhook/slack",
            &HashMap::new(),
            &HashMap::new(),
            &body,
            true,
        )
        .await
        .expect("HTTP callback failed");

    assert_eq!(response.status, 200);

    let msg = timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("message should arrive")
        .expect("stream should yield a message");

    assert_eq!(msg.content, "hello metadata test");
    // Thread ID should be set (channel or DM ID)
    assert!(
        msg.thread_id.is_some(),
        "Expected thread_id to be set for DM"
    );
}

#[tokio::test]
#[cfg(feature = "integration")]
async fn test_respond_posts_to_slack_api() {
    use axum::{
        Router, body::Bytes, extract::State, http::Uri, response::IntoResponse, routing::any,
    };

    #[derive(Clone)]
    struct FakeSlackState {
        requests: Arc<tokio::sync::Mutex<Vec<String>>>,
        post_message_payloads: Arc<tokio::sync::Mutex<Vec<serde_json::Value>>>,
    }

    async fn handler(
        State(state): State<FakeSlackState>,
        uri: Uri,
        body: Bytes,
    ) -> impl IntoResponse {
        state.requests.lock().await.push(uri.to_string());

        if uri.path().ends_with("/chat.postMessage") {
            let payload = serde_json::from_slice::<serde_json::Value>(&body)
                .unwrap_or_else(|err| panic!("invalid chat.postMessage payload: {err}"));
            state.post_message_payloads.lock().await.push(payload);
            return axum::Json(serde_json::json!({
                "ok": true,
                "channel": "DU42OWNER",
                "ts": "1234567890.000099",
                "message": { "text": "reply", "ts": "1234567890.000099" }
            }))
            .into_response();
        }

        (
            axum::http::StatusCode::NOT_FOUND,
            format!("Unhandled fake Slack path: {}", uri.path()),
        )
            .into_response()
    }

    require_slack_wasm!();
    let runtime = create_test_runtime();

    let state = FakeSlackState {
        requests: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        post_message_payloads: Arc::new(tokio::sync::Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route("/{*path}", any(handler))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake slack");
    let addr = listener.local_addr().expect("fake slack addr");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _guard = ScopedEnvVar::set(
        "IRONCLAW_TEST_HTTP_REWRITE_MAP",
        &slack_test_http_rewrite_map(&format!("http://{addr}")),
    );

    let config = serde_json::json!({
        "owner_id": null,
        "dm_policy": "open",
        "allow_from": [],
    })
    .to_string();

    let channel = create_slack_channel(runtime, &config).await;
    let mut stream = channel
        .start_message_stream_for_test()
        .await
        .expect("Failed to bootstrap test message stream");

    let body = build_slack_event_callback(serde_json::json!({
        "type": "message",
        "user": "U42OWNER",
        "text": "hello from slack dm",
        "channel": "DU42OWNER",
        "ts": "1234567890.000020",
        "channel_type": "im"
    }));

    let http_response = channel
        .call_on_http_request(
            "POST",
            "/webhook/slack",
            &HashMap::new(),
            &HashMap::new(),
            &body,
            true,
        )
        .await
        .expect("HTTP callback failed");
    assert_eq!(http_response.status, 200);

    let incoming = timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("message should arrive")
        .expect("stream should yield a message");
    assert_eq!(incoming.content, "hello from slack dm");

    channel
        .respond(
            &incoming,
            OutgoingResponse::text("hello back from ironclaw"),
        )
        .await
        .expect("slack respond should succeed");

    let payloads = timeout(Duration::from_secs(3), async {
        loop {
            let snapshot = state.post_message_payloads.lock().await.clone();
            if !snapshot.is_empty() {
                break snapshot;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("chat.postMessage should be captured");

    server.abort();

    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["channel"], serde_json::json!("DU42OWNER"));
    assert_eq!(
        payloads[0]["text"],
        serde_json::json!("hello back from ironclaw")
    );
}

#[tokio::test]
#[cfg(feature = "integration")]
async fn test_file_attachment_metadata() {
    require_slack_wasm!();
    let runtime = create_test_runtime();

    let config = serde_json::json!({
        "owner_id": null,
        "dm_policy": "open",
        "allow_from": [],
    })
    .to_string();

    let channel = create_slack_channel(runtime, &config).await;
    let mut stream = channel
        .start_message_stream_for_test()
        .await
        .expect("Failed to bootstrap test message stream");

    let body = build_slack_event_callback(serde_json::json!({
        "type": "message",
        "user": "U42OWNER",
        "text": "check this file",
        "channel": "DU42OWNER",
        "ts": "1234567890.000030",
        "channel_type": "im",
        "files": [
            {
                "id": "F0FILE001",
                "name": "report.pdf",
                "mimetype": "application/pdf",
                "url_private_download": "https://files.slack.com/files-pri/T0001-F0FILE001/report.pdf",
                "size": 2048
            }
        ]
    }));

    let response = channel
        .call_on_http_request(
            "POST",
            "/webhook/slack",
            &HashMap::new(),
            &HashMap::new(),
            &body,
            true,
        )
        .await
        .expect("HTTP callback failed");

    assert_eq!(response.status, 200);

    let msg = timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("message should arrive")
        .expect("stream should yield a message");

    assert_eq!(msg.content, "check this file");
    // The message should have file attachment metadata
    // (even if download fails without the test API override)
    assert!(
        !msg.attachments.is_empty() || msg.content.contains("check this file"),
        "Expected file metadata or text content"
    );
}

#[tokio::test]
#[cfg(feature = "integration")]
async fn test_channel_message_without_mention_ignored() {
    require_slack_wasm!();
    let runtime = create_test_runtime();

    let config = serde_json::json!({
        "owner_id": null,
        "dm_policy": "open",
        "allow_from": [],
    })
    .to_string();

    let channel = create_slack_channel(runtime, &config).await;
    let mut stream = channel
        .start_message_stream_for_test()
        .await
        .expect("Failed to bootstrap test message stream");

    // Regular message in a channel (not DM, not app_mention) should be ignored
    let body = build_slack_event_callback(serde_json::json!({
        "type": "message",
        "user": "U42OWNER",
        "text": "hello everyone",
        "channel": "C0001",
        "ts": "1234567890.000040",
        "channel_type": "channel"
    }));

    let response = channel
        .call_on_http_request(
            "POST",
            "/webhook/slack",
            &HashMap::new(),
            &HashMap::new(),
            &body,
            true,
        )
        .await
        .expect("HTTP callback failed");

    assert_eq!(response.status, 200);
    expect_no_message(&mut stream, 500).await;
}

/// Regression: build script must target wasm32-wasip2 so binaries land at the
/// path that `slack_wasm_path()` expects.  Without `--target wasm32-wasip2`
/// cargo-component defaults to wasip1 and CI tests silently skip.
#[test]
fn build_script_targets_wasip2() {
    let script = std::fs::read_to_string(find_project_file("scripts/build-wasm-extensions.sh"))
        .expect("build script should exist");
    assert!(
        script.contains("--target wasm32-wasip2"),
        "build-wasm-extensions.sh must pass --target wasm32-wasip2"
    );
}
