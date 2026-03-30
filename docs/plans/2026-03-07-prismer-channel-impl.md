# Prismer IM Channel Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a Prismer Cloud IM WASM channel to IronClaw with webhook + polling dual mode.

**Architecture:** Standalone WASM crate in `channels-src/prismer/` following the Telegram channel pattern. Webhook for real-time delivery (tunnel required), polling fallback (30s). Two-step auth: API key registers agent, JWT used for all subsequent IM calls.

**Tech Stack:** Rust (wasm32-wasip2), wit-bindgen 0.36, serde/serde_json, WIT channel interface.

**Design doc:** `docs/plans/2026-03-07-prismer-channel-design.md`

---

### Task 1: Scaffold WASM Crate

**Files:**
- Create: `channels-src/prismer/Cargo.toml`
- Create: `channels-src/prismer/src/lib.rs` (minimal skeleton)
- Create: `channels-src/prismer/build.sh`
- Create: `channels-src/prismer/prismer.capabilities.json`

**Step 1: Create Cargo.toml**

```toml
[package]
name = "prismer-channel"
version = "0.1.0"
edition = "2021"
description = "Prismer Cloud IM channel for IronClaw"
license = "MIT OR Apache-2.0"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

[profile.release]
opt-level = "s"
lto = true
strip = true
codegen-units = 1

[workspace]
```

**Step 2: Create minimal lib.rs with WIT bindings and empty callbacks**

```rust
#![allow(dead_code)]

//! Prismer Cloud IM channel for IronClaw.

wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};

use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusType, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};

struct PrismerChannel;
export_sandboxed_channel!(PrismerChannel);

impl Guest for PrismerChannel {
    fn on_start(_config_json: String) -> Result<ChannelConfig, String> {
        Err("not yet implemented".to_string())
    }

    fn on_http_request(_req: IncomingHttpRequest) -> OutgoingHttpResponse {
        OutgoingHttpResponse {
            status: 501,
            headers_json: "{}".to_string(),
            body: b"not implemented".to_vec(),
        }
    }

    fn on_poll() {}

    fn on_respond(_response: AgentResponse) -> Result<(), String> {
        Err("not yet implemented".to_string())
    }

    fn on_status(_update: StatusUpdate) {}

    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Prismer channel shutting down");
    }
}
```

**Step 3: Create capabilities.json**

Copy the full JSON from the design doc (`docs/plans/2026-03-07-prismer-channel-design.md`, line 132-193).

**Step 4: Create build.sh**

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

echo "Building Prismer channel WASM component..."

cargo build --release --target wasm32-wasip2

WASM_PATH="target/wasm32-wasip2/release/prismer_channel.wasm"

if [ -f "$WASM_PATH" ]; then
    wasm-tools component new "$WASM_PATH" -o prismer.wasm 2>/dev/null || cp "$WASM_PATH" prismer.wasm
    wasm-tools strip prismer.wasm -o prismer.wasm
    echo "Built: prismer.wasm ($(du -h prismer.wasm | cut -f1))"
    echo ""
    echo "To install:"
    echo "  mkdir -p ~/.ironclaw/channels"
    echo "  cp prismer.wasm prismer.capabilities.json ~/.ironclaw/channels/"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
```

**Step 5: Verify compilation**

Run: `cd channels-src/prismer && cargo check --target wasm32-wasip2`
Expected: Compiles without errors.

**Step 6: Commit**

```bash
git add channels-src/prismer/
git commit -m "feat(prismer): scaffold WASM channel crate with empty callbacks"
```

---

### Task 2: Types and Config Parsing

**Files:**
- Modify: `channels-src/prismer/src/lib.rs`

**Step 1: Write tests for config parsing**

Add at the bottom of `lib.rs`:

```rust
// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Deserialize)]
struct PrismerConfig {
    #[serde(default = "default_base_url")]
    base_url: String,
    agent_name: Option<String>,
    #[serde(default = "default_display_name")]
    display_name: String,
    #[serde(default = "default_agent_type")]
    agent_type: String,
    #[serde(default = "default_capabilities")]
    capabilities: Vec<String>,
    #[serde(default = "default_description")]
    description: String,
    #[serde(default)]
    polling_enabled: bool,
    #[serde(default = "default_poll_interval")]
    poll_interval_ms: u32,
    #[serde(default = "default_dm_policy")]
    dm_policy: String,
    /// Injected by host when a tunnel is active.
    tunnel_url: Option<String>,
}

fn default_base_url() -> String { "https://prismer.cloud".to_string() }
fn default_display_name() -> String { "IronClaw Agent".to_string() }
fn default_agent_type() -> String { "assistant".to_string() }
fn default_capabilities() -> Vec<String> {
    vec!["chat".into(), "code".into(), "memory".into(), "tools".into()]
}
fn default_description() -> String { "IronClaw AI assistant on Prismer network".to_string() }
fn default_poll_interval() -> u32 { 30000 }
fn default_dm_policy() -> String { "open".to_string() }

#[derive(Debug, Serialize, Deserialize)]
struct PrismerMetadata {
    conversation_id: String,
    #[serde(default)]
    conversation_type: String,
    sender_id: String,
    #[serde(default)]
    sender_username: String,
    message_id: String,
}

// -- Prismer API response types --

#[derive(Debug, Deserialize)]
struct IMResult {
    ok: bool,
    data: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RegisterData {
    #[serde(rename = "imUserId")]
    im_user_id: String,
    token: String,
    #[serde(rename = "expiresIn")]
    expires_in: Option<String>,
}

// -- Prismer webhook types (matches Go SDK WebhookPayload) --

#[derive(Debug, Deserialize)]
struct WebhookPayload {
    source: String,
    event: String,
    timestamp: Option<i64>,
    message: WebhookMessage,
    sender: WebhookSender,
    conversation: WebhookConversation,
}

#[derive(Debug, Deserialize)]
struct WebhookMessage {
    id: String,
    #[serde(rename = "type")]
    msg_type: String,
    content: String,
    #[serde(rename = "senderId")]
    sender_id: String,
    #[serde(rename = "conversationId")]
    conversation_id: String,
    #[serde(rename = "parentId")]
    parent_id: Option<String>,
    metadata: Option<serde_json::Value>,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WebhookSender {
    id: String,
    username: String,
    #[serde(rename = "displayName")]
    display_name: String,
    role: String,
}

#[derive(Debug, Deserialize)]
struct WebhookConversation {
    id: String,
    #[serde(rename = "type")]
    conv_type: String,
    title: Option<String>,
}

// -- IM message type (for polling) --

#[derive(Debug, Deserialize)]
struct IMMessage {
    id: String,
    content: String,
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(rename = "senderId")]
    sender_id: String,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IMConversation {
    id: String,
    #[serde(rename = "type")]
    conv_type: String,
    title: Option<String>,
    #[serde(rename = "unreadCount")]
    unread_count: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config_minimal() {
        let json = r#"{}"#;
        let config: PrismerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.base_url, "https://prismer.cloud");
        assert_eq!(config.display_name, "IronClaw Agent");
        assert_eq!(config.agent_type, "assistant");
        assert_eq!(config.capabilities, vec!["chat", "code", "memory", "tools"]);
        assert!(config.agent_name.is_none());
        assert!(config.tunnel_url.is_none());
        assert!(!config.polling_enabled);
        assert_eq!(config.poll_interval_ms, 30000);
    }

    #[test]
    fn test_parse_config_full() {
        let json = r#"{
            "base_url": "https://custom.prismer.dev",
            "agent_name": "my-bot",
            "display_name": "My Bot",
            "agent_type": "specialist",
            "capabilities": ["search"],
            "description": "A search bot",
            "polling_enabled": true,
            "poll_interval_ms": 60000,
            "dm_policy": "pairing",
            "tunnel_url": "https://abc.ngrok.io"
        }"#;
        let config: PrismerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.base_url, "https://custom.prismer.dev");
        assert_eq!(config.agent_name, Some("my-bot".to_string()));
        assert!(config.polling_enabled);
        assert_eq!(config.tunnel_url, Some("https://abc.ngrok.io".to_string()));
    }

    #[test]
    fn test_parse_webhook_payload() {
        let json = r#"{
            "source": "prismer_im",
            "event": "message.new",
            "timestamp": 1741334400,
            "message": {
                "id": "msg_001",
                "type": "text",
                "content": "Hello from Prismer",
                "senderId": "iu_user_123",
                "conversationId": "conv_abc",
                "parentId": null,
                "metadata": {},
                "createdAt": "2026-03-07T10:00:00Z"
            },
            "sender": {
                "id": "iu_user_123",
                "username": "william",
                "displayName": "William",
                "role": "human"
            },
            "conversation": {
                "id": "conv_abc",
                "type": "direct",
                "title": null
            }
        }"#;
        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.source, "prismer_im");
        assert_eq!(payload.event, "message.new");
        assert_eq!(payload.message.content, "Hello from Prismer");
        assert_eq!(payload.sender.username, "william");
        assert_eq!(payload.conversation.conv_type, "direct");
    }

    #[test]
    fn test_parse_webhook_invalid_source() {
        let json = r#"{
            "source": "other_system",
            "event": "message.new",
            "message": {"id":"m","type":"text","content":"x","senderId":"s","conversationId":"c"},
            "sender": {"id":"s","username":"u","displayName":"U","role":"human"},
            "conversation": {"id":"c","type":"direct","title":null}
        }"#;
        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert_ne!(payload.source, "prismer_im");
    }

    #[test]
    fn test_parse_webhook_missing_fields() {
        let json = r#"{"source": "prismer_im"}"#;
        let result = serde_json::from_str::<WebhookPayload>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_metadata_roundtrip() {
        let meta = PrismerMetadata {
            conversation_id: "conv_123".to_string(),
            conversation_type: "direct".to_string(),
            sender_id: "iu_user".to_string(),
            sender_username: "alice".to_string(),
            message_id: "msg_456".to_string(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: PrismerMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(back.conversation_id, "conv_123");
        assert_eq!(back.message_id, "msg_456");
    }

    #[test]
    fn test_parse_register_response() {
        let json = r#"{"imUserId":"iu_xxx","token":"jwt_abc","expiresIn":"24h"}"#;
        let data: RegisterData = serde_json::from_str(json).unwrap();
        assert_eq!(data.im_user_id, "iu_xxx");
        assert_eq!(data.token, "jwt_abc");
        assert_eq!(data.expires_in, Some("24h".to_string()));
    }

    #[test]
    fn test_parse_im_result_ok() {
        let json = r#"{"ok":true,"data":{"token":"abc"}}"#;
        let result: IMResult = serde_json::from_str(json).unwrap();
        assert!(result.ok);
        assert!(result.data.is_some());
    }

    #[test]
    fn test_parse_im_result_error() {
        let json = r#"{"ok":false,"error":{"code":"UNAUTHORIZED","message":"bad token"}}"#;
        let result: IMResult = serde_json::from_str(json).unwrap();
        assert!(!result.ok);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_skip_self_message() {
        let my_id = "iu_bot";
        let sender_id = "iu_bot";
        assert_eq!(my_id, sender_id, "Self-messages should be skipped");
    }

    #[test]
    fn test_build_send_payload() {
        let body = serde_json::json!({
            "content": "Hello **world**",
            "type": "markdown",
        });
        let obj = body.as_object().unwrap();
        assert_eq!(obj.get("content").unwrap(), "Hello **world**");
        assert_eq!(obj.get("type").unwrap(), "markdown");
    }
}
```

**Step 2: Run tests (native target, not WASM)**

Run: `cd channels-src/prismer && cargo test`
Expected: All tests pass.

**Step 3: Commit**

```bash
git add channels-src/prismer/src/lib.rs
git commit -m "feat(prismer): add types, config parsing, and unit tests"
```

---

### Task 3: HTTP Helpers and Auth

**Files:**
- Modify: `channels-src/prismer/src/lib.rs`

**Step 1: Add HTTP helper functions and auth logic**

Add above the `impl Guest` block:

```rust
// ============================================================================
// Workspace State Paths
// ============================================================================

const JWT_PATH: &str = "jwt";
const IM_USER_ID_PATH: &str = "im_user_id";
const CONFIG_PATH: &str = "config";
const WEBHOOK_REGISTERED_PATH: &str = "webhook_registered";

// ============================================================================
// HTTP Helpers
// ============================================================================

fn respond_json(status: u16, body: serde_json::Value) -> OutgoingHttpResponse {
    OutgoingHttpResponse {
        status,
        headers_json: r#"{"Content-Type":"application/json"}"#.to_string(),
        body: serde_json::to_vec(&body).unwrap_or_default(),
    }
}

fn api_request(
    method: &str,
    url: &str,
    token: &str,
    body: Option<&serde_json::Value>,
) -> Result<(u16, Vec<u8>), String> {
    let headers = if body.is_some() {
        serde_json::json!({
            "Authorization": format!("Bearer {}", token),
            "Content-Type": "application/json"
        })
    } else {
        serde_json::json!({
            "Authorization": format!("Bearer {}", token)
        })
    };

    let body_bytes = body.map(|b| serde_json::to_vec(b).unwrap_or_default());

    let resp = channel_host::http_request(
        method,
        url,
        &headers.to_string(),
        body_bytes.as_deref(),
        None,
    )
    .map_err(|e| format!("HTTP request failed: {}", e))?;

    Ok((resp.status, resp.body))
}

fn read_config() -> PrismerConfig {
    channel_host::workspace_read(CONFIG_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| PrismerConfig {
            base_url: default_base_url(),
            agent_name: None,
            display_name: default_display_name(),
            agent_type: default_agent_type(),
            capabilities: default_capabilities(),
            description: default_description(),
            polling_enabled: false,
            poll_interval_ms: default_poll_interval(),
            dm_policy: default_dm_policy(),
            tunnel_url: None,
        })
}

// ============================================================================
// Authentication
// ============================================================================

fn register_agent(config: &PrismerConfig) -> Result<(String, String), String> {
    let body = serde_json::json!({
        "type": "agent",
        "username": config.agent_name.as_deref().unwrap_or("ironclaw"),
        "displayName": config.display_name,
        "agentType": config.agent_type,
        "capabilities": config.capabilities,
        "description": config.description,
        "endpoint": config.tunnel_url,
    });

    // Use placeholder — host replaces {PRISMER_API_KEY} with actual secret value
    let (status, resp_body) = api_request(
        "POST",
        &format!("{}/api/im/register", config.base_url),
        "{PRISMER_API_KEY}",
        Some(&body),
    )?;

    if status >= 300 {
        let err_text = String::from_utf8_lossy(&resp_body);
        return Err(format!("Register failed (HTTP {}): {}", status, err_text));
    }

    let im_result: IMResult = serde_json::from_slice(&resp_body)
        .map_err(|e| format!("Failed to parse register response: {}", e))?;

    if !im_result.ok {
        return Err(format!("Register returned ok=false: {:?}", im_result.error));
    }

    let data: RegisterData = serde_json::from_value(
        im_result.data.ok_or("Register response missing data")?
    ).map_err(|e| format!("Failed to parse register data: {}", e))?;

    // Persist JWT and user ID
    channel_host::workspace_write(JWT_PATH, &data.token)
        .map_err(|e| format!("Failed to write JWT: {}", e))?;
    channel_host::workspace_write(IM_USER_ID_PATH, &data.im_user_id)
        .map_err(|e| format!("Failed to write user ID: {}", e))?;

    channel_host::log(
        channel_host::LogLevel::Info,
        &format!("Registered as {} ({})", data.im_user_id,
                 if data.expires_in.is_some() { "new" } else { "existing" }),
    );

    Ok((data.token, data.im_user_id))
}

fn ensure_jwt(config: &PrismerConfig) -> Result<String, String> {
    // Try cached JWT first
    if let Some(cached) = channel_host::workspace_read(JWT_PATH) {
        if !cached.is_empty() {
            // Validate with /api/im/me
            let (status, _) = api_request(
                "GET",
                &format!("{}/api/im/me", config.base_url),
                &cached,
                None,
            )?;

            if status < 300 {
                channel_host::log(channel_host::LogLevel::Debug, "Cached JWT is valid");
                return Ok(cached);
            }

            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Cached JWT invalid (HTTP {}), re-registering", status),
            );
        }
    }

    // Register to get fresh JWT
    let (jwt, _) = register_agent(config)?;
    Ok(jwt)
}

/// Re-register and return fresh JWT. Used for 401 recovery in on_respond/on_poll.
fn attempt_re_register() -> Result<String, String> {
    let config = read_config();
    let (jwt, _) = register_agent(&config)?;
    Ok(jwt)
}
```

**Step 2: Verify compilation**

Run: `cd channels-src/prismer && cargo check --target wasm32-wasip2`
Expected: Compiles without errors.

**Step 3: Commit**

```bash
git add channels-src/prismer/src/lib.rs
git commit -m "feat(prismer): add HTTP helpers, auth flow, JWT caching"
```

---

### Task 4: Implement on_start

**Files:**
- Modify: `channels-src/prismer/src/lib.rs`

**Step 1: Replace the stub on_start with full implementation**

```rust
fn on_start(config_json: String) -> Result<ChannelConfig, String> {
    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!("Prismer channel config: {}", config_json),
    );

    let config: PrismerConfig = serde_json::from_str(&config_json)
        .map_err(|e| format!("Failed to parse config: {}", e))?;

    // Persist config for subsequent callbacks
    let config_str = serde_json::to_string(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    let _ = channel_host::workspace_write(CONFIG_PATH, &config_str);

    channel_host::log(channel_host::LogLevel::Info, "Prismer channel starting");

    // Authenticate (cached JWT or fresh register)
    let _jwt = ensure_jwt(&config)?;

    // Determine mode
    let webhook_mode = config.tunnel_url.is_some();

    if webhook_mode {
        channel_host::log(
            channel_host::LogLevel::Info,
            "Webhook mode enabled (tunnel configured)",
        );
        if let Some(ref tunnel_url) = config.tunnel_url {
            let _ = channel_host::workspace_write(
                WEBHOOK_REGISTERED_PATH,
                &format!("{}/webhook/prismer", tunnel_url),
            );
        }
    } else {
        channel_host::log(
            channel_host::LogLevel::Info,
            "Polling mode enabled (no tunnel configured)",
        );
    }

    let poll = if !webhook_mode {
        Some(PollConfig {
            interval_ms: config.poll_interval_ms.max(30000),
            enabled: true,
        })
    } else {
        None
    };

    Ok(ChannelConfig {
        display_name: "Prismer".to_string(),
        http_endpoints: vec![HttpEndpointConfig {
            path: "/webhook/prismer".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: channel_host::secret_exists("prismer_webhook_secret"),
        }],
        poll,
    })
}
```

**Step 2: Verify compilation**

Run: `cd channels-src/prismer && cargo check --target wasm32-wasip2`
Expected: Compiles.

**Step 3: Commit**

```bash
git add channels-src/prismer/src/lib.rs
git commit -m "feat(prismer): implement on_start with auth and mode detection"
```

---

### Task 5: Implement on_http_request (Webhook)

**Files:**
- Modify: `channels-src/prismer/src/lib.rs`

**Step 1: Replace the stub on_http_request**

```rust
fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
    if !req.secret_validated {
        channel_host::log(
            channel_host::LogLevel::Warn,
            "Webhook request with invalid or missing signature",
        );
        return respond_json(401, serde_json::json!({"error": "Invalid signature"}));
    }

    let payload: WebhookPayload = match serde_json::from_slice(&req.body) {
        Ok(p) => p,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to parse webhook payload: {}", e),
            );
            return respond_json(400, serde_json::json!({"error": e.to_string()}));
        }
    };

    if payload.source != "prismer_im" {
        channel_host::log(
            channel_host::LogLevel::Warn,
            &format!("Unknown webhook source: {}", payload.source),
        );
        return respond_json(400, serde_json::json!({"error": "Unknown source"}));
    }

    // Skip self-messages
    let my_id = channel_host::workspace_read(IM_USER_ID_PATH).unwrap_or_default();
    if payload.sender.id == my_id {
        return respond_json(200, serde_json::json!({"ok": true, "skipped": "self"}));
    }

    // Only process new messages
    if payload.event != "message.new" {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Ignoring event: {}", payload.event),
        );
        return respond_json(200, serde_json::json!({"ok": true}));
    }

    // Skip empty content
    let content = payload.message.content.trim();
    if content.is_empty() {
        return respond_json(200, serde_json::json!({"ok": true, "skipped": "empty"}));
    }

    let metadata = PrismerMetadata {
        conversation_id: payload.conversation.id.clone(),
        conversation_type: payload.conversation.conv_type.clone(),
        sender_id: payload.sender.id.clone(),
        sender_username: payload.sender.username.clone(),
        message_id: payload.message.id.clone(),
    };

    channel_host::emit_message(EmittedMessage {
        user_id: payload.sender.id,
        user_name: Some(payload.sender.display_name),
        content: content.to_string(),
        thread_id: Some(payload.conversation.id),
        metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
    });

    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!("Emitted message {} from {}", metadata.message_id, metadata.sender_username),
    );

    respond_json(200, serde_json::json!({"ok": true}))
}
```

**Step 2: Verify compilation**

Run: `cd channels-src/prismer && cargo check --target wasm32-wasip2`

**Step 3: Commit**

```bash
git add channels-src/prismer/src/lib.rs
git commit -m "feat(prismer): implement on_http_request webhook handler"
```

---

### Task 6: Implement on_respond

**Files:**
- Modify: `channels-src/prismer/src/lib.rs`

**Step 1: Replace the stub on_respond**

```rust
fn on_respond(response: AgentResponse) -> Result<(), String> {
    let metadata: PrismerMetadata = serde_json::from_str(&response.metadata_json)
        .map_err(|e| format!("Failed to parse metadata: {}", e))?;

    let jwt = channel_host::workspace_read(JWT_PATH)
        .filter(|t| !t.is_empty())
        .ok_or("No JWT token available")?;
    let config = read_config();

    let body = serde_json::json!({
        "content": response.content,
        "type": "markdown",
    });

    let url = format!("{}/api/im/messages/{}", config.base_url, metadata.conversation_id);

    let (status, resp_body) = api_request("POST", &url, &jwt, Some(&body))?;

    match status {
        s if s < 300 => {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Sent reply to conversation {}", metadata.conversation_id),
            );
            Ok(())
        }
        401 => {
            channel_host::log(
                channel_host::LogLevel::Info,
                "JWT expired during on_respond, re-registering",
            );
            let new_jwt = attempt_re_register()?;
            let (retry_status, retry_body) = api_request("POST", &url, &new_jwt, Some(&body))?;
            if retry_status < 300 {
                Ok(())
            } else {
                let err_text = String::from_utf8_lossy(&retry_body);
                Err(format!("Send failed after re-register (HTTP {}): {}", retry_status, err_text))
            }
        }
        _ => {
            let err_text = String::from_utf8_lossy(&resp_body);
            Err(format!("Send failed (HTTP {}): {}", status, err_text))
        }
    }
}
```

**Step 2: Verify compilation**

Run: `cd channels-src/prismer && cargo check --target wasm32-wasip2`

**Step 3: Commit**

```bash
git add channels-src/prismer/src/lib.rs
git commit -m "feat(prismer): implement on_respond with JWT retry"
```

---

### Task 7: Implement on_poll

**Files:**
- Modify: `channels-src/prismer/src/lib.rs`

**Step 1: Implement on_poll**

```rust
fn on_poll() {
    let jwt = match channel_host::workspace_read(JWT_PATH).filter(|t| !t.is_empty()) {
        Some(t) => t,
        None => {
            channel_host::log(channel_host::LogLevel::Warn, "No JWT, skipping poll");
            return;
        }
    };

    let config = read_config();
    let my_id = channel_host::workspace_read(IM_USER_ID_PATH).unwrap_or_default();

    // Fetch conversations with unread messages
    let conv_url = format!("{}/api/im/conversations?withUnread=true", config.base_url);
    let (status, body) = match api_request("GET", &conv_url, &jwt, None) {
        Ok(r) => r,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to fetch conversations: {}", e),
            );
            return;
        }
    };

    if status == 401 {
        channel_host::log(channel_host::LogLevel::Info, "JWT expired during poll, re-registering");
        let _ = attempt_re_register();
        return;
    }

    if status >= 300 {
        channel_host::log(
            channel_host::LogLevel::Error,
            &format!("Conversations fetch failed (HTTP {})", status),
        );
        return;
    }

    let im_result: IMResult = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to parse conversations response: {}", e),
            );
            return;
        }
    };

    let conversations: Vec<IMConversation> = match im_result.data {
        Some(data) => serde_json::from_value(data).unwrap_or_default(),
        None => return,
    };

    let jwt_for_requests = match channel_host::workspace_read(JWT_PATH).filter(|t| !t.is_empty()) {
        Some(t) => t,
        None => return,
    };

    for conv in &conversations {
        let unread = conv.unread_count.unwrap_or(0);
        if unread <= 0 {
            continue;
        }

        let cursor_key = format!("cursor_{}", conv.id);
        let cursor = channel_host::workspace_read(&cursor_key).unwrap_or_default();

        let mut msg_url = format!("{}/api/im/messages/{}", config.base_url, conv.id);
        if !cursor.is_empty() {
            msg_url.push_str(&format!("?offset={}", cursor));
        }

        let (msg_status, msg_body) = match api_request("GET", &msg_url, &jwt_for_requests, None) {
            Ok(r) => r,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to fetch messages for {}: {}", conv.id, e),
                );
                continue;
            }
        };

        if msg_status >= 300 {
            continue;
        }

        let msg_result: IMResult = match serde_json::from_slice(&msg_body) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let messages: Vec<IMMessage> = match msg_result.data {
            Some(data) => serde_json::from_value(data).unwrap_or_default(),
            None => continue,
        };

        let mut new_offset = cursor.clone();

        for msg in &messages {
            if msg.sender_id == my_id {
                continue;
            }

            let content = msg.content.trim();
            if content.is_empty() {
                continue;
            }

            let metadata = PrismerMetadata {
                conversation_id: conv.id.clone(),
                conversation_type: conv.conv_type.clone(),
                sender_id: msg.sender_id.clone(),
                sender_username: String::new(),
                message_id: msg.id.clone(),
            };

            channel_host::emit_message(EmittedMessage {
                user_id: msg.sender_id.clone(),
                user_name: None,
                content: content.to_string(),
                thread_id: Some(conv.id.clone()),
                metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
            });
        }

        // Update cursor (use message count as offset)
        if !messages.is_empty() {
            let current_offset: usize = cursor.parse().unwrap_or(0);
            new_offset = (current_offset + messages.len()).to_string();
            let _ = channel_host::workspace_write(&cursor_key, &new_offset);
        }

        // Mark as read
        let read_url = format!(
            "{}/api/im/conversations/{}/read",
            config.base_url, conv.id
        );
        let _ = api_request("POST", &read_url, &jwt_for_requests, None);
    }
}
```

**Step 2: Verify compilation**

Run: `cd channels-src/prismer && cargo check --target wasm32-wasip2`

**Step 3: Commit**

```bash
git add channels-src/prismer/src/lib.rs
git commit -m "feat(prismer): implement on_poll with cursor tracking"
```

---

### Task 8: Implement on_status and on_shutdown

**Files:**
- Modify: `channels-src/prismer/src/lib.rs`

**Step 1: Replace stubs**

```rust
fn on_status(update: StatusUpdate) {
    // Prismer has no HTTP typing endpoint (typing is WebSocket-only).
    // Log for debugging; extensible later.
    if matches!(update.status, StatusType::Thinking) {
        channel_host::log(
            channel_host::LogLevel::Debug,
            "Agent thinking (Prismer has no HTTP typing API)",
        );
    }
}

fn on_shutdown() {
    channel_host::log(
        channel_host::LogLevel::Info,
        "Prismer channel shutting down",
    );
}
```

**Step 2: Verify compilation and run tests**

Run: `cd channels-src/prismer && cargo check --target wasm32-wasip2 && cargo test`
Expected: Both pass.

**Step 3: Commit**

```bash
git add channels-src/prismer/src/lib.rs
git commit -m "feat(prismer): implement on_status and on_shutdown"
```

---

### Task 9: Host-Side Registration

**Files:**
- Modify: `src/channels/wasm/bundled.rs:20-25`

**Step 1: Add prismer to KNOWN_CHANNELS**

Change line 20-25 from:

```rust
const KNOWN_CHANNELS: &[(&str, &str)] = &[
    ("telegram", "telegram_channel"),
    ("slack", "slack_channel"),
    ("discord", "discord_channel"),
    ("whatsapp", "whatsapp_channel"),
];
```

To:

```rust
const KNOWN_CHANNELS: &[(&str, &str)] = &[
    ("telegram", "telegram_channel"),
    ("slack", "slack_channel"),
    ("discord", "discord_channel"),
    ("whatsapp", "whatsapp_channel"),
    ("prismer", "prismer_channel"),
];
```

**Step 2: Update the test on line 144-151**

Change:

```rust
fn test_known_channels_includes_all_four() {
```

To:

```rust
fn test_known_channels_includes_all() {
```

And add:

```rust
assert!(names.contains(&"prismer"));
```

**Step 3: Run host tests**

Run: `cargo test channels::wasm::bundled`
Expected: Pass (with updated assertion).

**Step 4: Run clippy**

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: Zero warnings.

**Step 5: Commit**

```bash
git add src/channels/wasm/bundled.rs
git commit -m "feat(prismer): register prismer in KNOWN_CHANNELS"
```

---

### Task 10: Build WASM Binary and Full Verification

**Step 1: Build the WASM binary**

Run: `cd channels-src/prismer && chmod +x build.sh && bash build.sh`
Expected: `prismer.wasm` created successfully.

**Step 2: Run all prismer unit tests**

Run: `cd channels-src/prismer && cargo test -- --nocapture`
Expected: All pass.

**Step 3: Run host clippy**

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: Zero warnings.

**Step 4: Run host tests**

Run: `cargo test`
Expected: All pass (including bundled channel tests).

**Step 5: Commit .gitignore for build artifacts**

The `target/` and `prismer.wasm` build output should not be committed.
Verify `channels-src/prismer/target/` is gitignored (the parent `.gitignore`
should already handle `target/`).

**Step 6: Final commit**

```bash
git add -A
git commit -m "feat(prismer): Prismer Cloud IM WASM channel complete

WASM channel supporting webhook + polling dual mode for Prismer Cloud IM.
- Webhook mode: real-time message delivery via X-Prismer-Signature signed POSTs
- Polling mode: 30s interval fallback when no tunnel configured
- Two-step auth: API key register -> JWT for subsequent calls
- Self-message loop prevention
- JWT auto-renewal on 401

Generated with [Claude Code](https://claude.ai/code)
via [Happy](https://happy.engineering)

Co-Authored-By: Claude <noreply@anthropic.com>
Co-Authored-By: Happy <yesreply@happy.engineering>"
```

**Step 7: Push to fork**

Run: `git push fork feat/prismer-channel`

---

## Summary

| Task | What | Files |
|------|------|-------|
| 1 | Scaffold crate | `Cargo.toml`, `lib.rs` (skeleton), `build.sh`, `capabilities.json` |
| 2 | Types + tests | `lib.rs` (types, 11 unit tests) |
| 3 | HTTP helpers + auth | `lib.rs` (api_request, register_agent, ensure_jwt) |
| 4 | on_start | `lib.rs` (config, auth, mode detection) |
| 5 | on_http_request | `lib.rs` (webhook handler) |
| 6 | on_respond | `lib.rs` (send reply with JWT retry) |
| 7 | on_poll | `lib.rs` (conversation polling with cursors) |
| 8 | on_status + on_shutdown | `lib.rs` (no-op + log) |
| 9 | Host registration | `bundled.rs` (KNOWN_CHANNELS + test) |
| 10 | Build + verify | WASM binary, clippy, all tests |
