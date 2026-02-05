# Building WASM Channels

This guide covers how to build WASM channel modules for IronClaw.

## Overview

Channels are WASM components that handle communication with external messaging platforms (Telegram, WhatsApp, Slack, etc.). They run in a sandboxed environment and communicate with the host via the WIT (WebAssembly Interface Types) interface.

## Directory Structure

```
channels/                    # Or channels-src/
└── my-channel/
    ├── Cargo.toml
    ├── src/
    │   └── lib.rs
    └── my-channel.capabilities.json
```

After building, deploy to:
```
~/.ironclaw/channels/
├── my-channel.wasm
└── my-channel.capabilities.json
```

## Cargo.toml Template

```toml
[package]
name = "my-channel"
version = "0.1.0"
edition = "2021"
description = "My messaging platform channel for IronClaw"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[profile.release]
opt-level = "s"
lto = true
strip = true
codegen-units = 1
```

## Channel Implementation

### Required Imports

```rust
// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",  // Adjust path as needed
});

use serde::{Deserialize, Serialize};

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig,
};
use near::agent::channel_host::{self, EmittedMessage};
```

### Implementing the Guest Trait

```rust
struct MyChannel;

impl Guest for MyChannel {
    /// Called once when the channel starts.
    /// Returns configuration for webhooks and polling.
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        // Parse config from capabilities file
        let config: MyConfig = serde_json::from_str(&config_json)
            .unwrap_or_default();

        Ok(ChannelConfig {
            display_name: "My Channel".to_string(),
            http_endpoints: vec![
                HttpEndpointConfig {
                    path: "/webhook/my-channel".to_string(),
                    methods: vec!["POST".to_string()],
                    require_secret: true,  // Validate webhook secret
                },
            ],
            poll: None,  // Or Some(PollConfig { interval_ms, enabled })
        })
    }

    /// Handle incoming HTTP requests (webhooks).
    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Parse webhook payload
        // Emit messages to agent
        // Return response to webhook caller
    }

    /// Called periodically if polling is enabled.
    fn on_poll() {
        // Fetch new messages from API
        // Emit any new messages
    }

    /// Send a response back to the messaging platform.
    fn on_respond(response: AgentResponse) -> Result<(), String> {
        // Parse metadata to get routing info
        // Call platform API to send message
    }

    /// Called when channel is shutting down.
    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Channel shutting down");
    }
}

// Export the channel implementation
export!(MyChannel);
```

## Critical Pattern: Metadata Flow

**The most important pattern**: Store routing info in message metadata so responses can be delivered.

```rust
// When receiving a message, store routing info:
#[derive(Debug, Serialize, Deserialize)]
struct MyMessageMetadata {
    chat_id: String,           // Where to send response
    sender_id: String,         // Who sent it (becomes recipient)
    original_message_id: String,
}

// In on_http_request or on_poll:
let metadata = MyMessageMetadata {
    chat_id: message.chat.id.clone(),
    sender_id: message.from.clone(),  // CRITICAL: Store sender!
    original_message_id: message.id.clone(),
};

channel_host::emit_message(&EmittedMessage {
    user_id: message.from.clone(),
    user_name: Some(name),
    content: text,
    thread_id: None,
    metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
});

// In on_respond, use the ORIGINAL message's metadata:
fn on_respond(response: AgentResponse) -> Result<(), String> {
    let metadata: MyMessageMetadata = serde_json::from_str(&response.metadata_json)?;

    // sender_id becomes the recipient!
    send_message(metadata.chat_id, metadata.sender_id, response.content);
}
```

## Credential Injection

**Never hardcode credentials!** Use placeholders that the host replaces:

### URL Placeholders (Telegram-style)

```rust
// The host replaces {TELEGRAM_BOT_TOKEN} with the actual token
let url = "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/sendMessage";
channel_host::http_request("POST", url, &headers_json, Some(&body));
```

### Header Placeholders (WhatsApp-style)

```rust
// The host replaces {WHATSAPP_ACCESS_TOKEN} in headers too
let headers = serde_json::json!({
    "Content-Type": "application/json",
    "Authorization": "Bearer {WHATSAPP_ACCESS_TOKEN}"
});
channel_host::http_request("POST", &url, &headers.to_string(), Some(&body));
```

The placeholder format is `{SECRET_NAME}` where `SECRET_NAME` matches the credential name in uppercase with underscores (e.g., `whatsapp_access_token` → `{WHATSAPP_ACCESS_TOKEN}`).

## Capabilities File

Create `my-channel.capabilities.json`:

```json
{
  "type": "channel",
  "name": "my-channel",
  "description": "My messaging platform channel",
  "setup": {
    "required_secrets": [
      {
        "name": "my_channel_api_token",
        "prompt": "Enter your API token",
        "validation": "^[A-Za-z0-9_-]+$"
      },
      {
        "name": "my_channel_webhook_secret",
        "prompt": "Webhook secret (leave empty to auto-generate)",
        "optional": true,
        "auto_generate": { "length": 32 }
      }
    ],
    "validation_endpoint": "https://api.my-platform.com/verify?token={my_channel_api_token}"
  },
  "capabilities": {
    "http": {
      "allowlist": [
        { "host": "api.my-platform.com", "path_prefix": "/" }
      ],
      "rate_limit": {
        "requests_per_minute": 60,
        "requests_per_hour": 1000
      }
    },
    "secrets": {
      "allowed_names": ["my_channel_*"]
    },
    "channel": {
      "allowed_paths": ["/webhook/my-channel"],
      "allow_polling": false,
      "workspace_prefix": "channels/my-channel/",
      "emit_rate_limit": {
        "messages_per_minute": 100,
        "messages_per_hour": 5000
      },
      "webhook": {
        "secret_header": "X-Webhook-Secret",
        "secret_name": "my_channel_webhook_secret"
      }
    }
  },
  "config": {
    "custom_option": "value"
  }
}
```

## Building and Deploying

```bash
# Build the WASM component
cd channels/my-channel
cargo component build --release

# Deploy to ~/.ironclaw/channels/
cp target/wasm32-wasip1/release/my_channel.wasm ~/.ironclaw/channels/my-channel.wasm
cp my-channel.capabilities.json ~/.ironclaw/channels/
```

## Host Functions Available

The channel host provides these functions:

```rust
// Logging
channel_host::log(LogLevel::Info, "Message");

// Time
let now = channel_host::now_millis();

// Workspace (scoped to channel namespace)
let data = channel_host::workspace_read("state/offset");
channel_host::workspace_write("state/offset", "12345")?;

// HTTP requests (credentials auto-injected)
let response = channel_host::http_request("POST", &url, &headers, Some(&body))?;

// Emit message to agent
channel_host::emit_message(&EmittedMessage { ... });
```

## Common Patterns

### Webhook Secret Validation

The host validates webhook secrets automatically. Check `req.secret_validated`:

```rust
fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
    if !req.secret_validated {
        channel_host::log(LogLevel::Warn, "Invalid webhook secret");
        // Host should have already rejected, but defense in depth
    }
    // ...
}
```

### Polling with Offset Tracking

For platforms that require polling (not webhook-based):

```rust
const OFFSET_PATH: &str = "state/last_offset";

fn on_poll() {
    // Read last offset
    let offset = channel_host::workspace_read(OFFSET_PATH)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);

    // Fetch updates since offset
    let updates = fetch_updates(offset);

    // Process and track new offset
    let mut new_offset = offset;
    for update in updates {
        if update.id >= new_offset {
            new_offset = update.id + 1;
        }
        emit_message(update);
    }

    // Save new offset
    if new_offset != offset {
        let _ = channel_host::workspace_write(OFFSET_PATH, &new_offset.to_string());
    }
}
```

### Status Message Filtering

Skip status updates to prevent loops:

```rust
// Skip status updates (delivered, read, etc.)
if !payload.statuses.is_empty() && payload.messages.is_empty() {
    return;  // Only status updates, no actual messages
}
```

### Bot Message Filtering

Skip bot messages to prevent infinite loops:

```rust
if sender.is_bot {
    return;  // Don't respond to bots
}
```

## Testing

Add tests in the same file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_webhook() {
        let json = r#"{ ... }"#;
        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.messages.len(), 1);
    }

    #[test]
    fn test_metadata_roundtrip() {
        let meta = MyMessageMetadata { ... };
        let json = serde_json::to_string(&meta).unwrap();
        let parsed: MyMessageMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(meta.chat_id, parsed.chat_id);
    }
}
```

Run tests with:
```bash
cargo test
```

## Troubleshooting

### "byte index N is not a char boundary"

Never slice strings by byte index! Use character-aware truncation:

```rust
// BAD: panics on multi-byte UTF-8 (emoji, etc.)
let preview = &content[..50];

// GOOD: safe truncation
let preview: String = content.chars().take(50).collect();
```

### Credential placeholders not replaced

1. Check the secret name matches (lowercase with underscores)
2. Verify the secret is in `allowed_names` in capabilities
3. Check logs for "unresolved placeholders" warnings

### Messages not routing to responses

Ensure `on_respond` uses the ORIGINAL message's metadata, not response metadata:
```rust
// response.metadata_json comes from the ORIGINAL emit_message call
let metadata: MyMetadata = serde_json::from_str(&response.metadata_json)?;
```
