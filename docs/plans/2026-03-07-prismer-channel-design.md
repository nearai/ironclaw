# Prismer IM Channel Design

## Overview

Add a Prismer Cloud IM channel to IronClaw as a WASM component, following the same
pattern as the existing Telegram channel. This enables IronClaw agents to send and
receive messages on the Prismer network for agent-to-agent and human-to-agent
communication.

**Prismer Cloud** provides an inter-agent messaging system with real-time WebSocket/SSE
support, agent discovery, and knowledge tools. The official Go SDK lives at
`github.com/Prismer-AI/Prismer/sdk/golang`.

## Decision: WASM + Webhook/Polling Dual Mode

### Why WASM (not native)

- Telegram, Slack, Discord, WhatsApp channels are all WASM — this is the project convention
- WASM provides sandboxed execution with credential zero-exposure
- Independent compilation and distribution
- No changes to main IronClaw binary

### Why not WebSocket

The WIT interface (`wit/channel.wit`) does not expose WebSocket host functions.
WASM channels use a callback model (`on_http_request`, `on_poll`, `on_respond`,
`on_status`) with fresh instances per callback — no way to hold a long-lived connection.

Webhook mode provides equivalent real-time delivery (millisecond latency from
Prismer Cloud), and polling serves as a fallback for deployments without a public URL.

## Architecture

### File Structure

```
channels-src/prismer/
  Cargo.toml                    # Standalone WASM crate (cdylib)
  build.sh                      # Build script (wasm32-wasip2)
  prismer.capabilities.json     # Capability declaration
  src/
    lib.rs                      # Full implementation (single file)
```

Host-side change:

```
src/channels/wasm/bundled.rs    # Add ("prismer", "prismer_channel") to KNOWN_CHANNELS
```

### Data Flow

**Webhook mode (tunnel configured):**

```
Prismer Cloud                    IronClaw Host              WASM (on_http_request)
     |                                |                              |
     |  POST /webhook/prismer         |                              |
     |  X-Prismer-Signature: sha256=  |                              |
     |  {source,event,message,sender} |                              |
     | ------------------------------>|  route to WASM               |
     |                                |----------------------------->|
     |                                |  verify signature            |
     |                                |  parse WebhookPayload        |
     |                                |  skip self-messages           |
     |                                |  emit-message()              |
     |                                |<-----------------------------|
     |  200 OK                        |                              |
     |<-------------------------------|                              |
     |                                |                              |
     |                                |  Agent processes, calls      |
     |                                |  on_respond()                |
     |                                |----------------------------->|
     |  POST /api/im/messages/{id}    |  workspace-read("jwt")       |
     |  Authorization: Bearer jwt     |  http-request()              |
     |<-------------------------------|<-----------------------------|
```

**Polling mode (no tunnel):**

```
     |                                |  Timer fires (30s)           |
     |                                |----------------------------->|
     |                                |  on_poll()                   |
     |  GET /api/im/conversations     |  workspace-read("jwt")      |
     |  ?withUnread=true              |  http-request()              |
     |<-------------------------------|<-----------------------------|
     |  {conversations}               |                              |
     |                                |  For each unread conv:       |
     |  GET /api/im/messages/{id}     |  http-request()              |
     |  ?offset=cursor                |                              |
     |<-------------------------------|<-----------------------------|
     |  {messages}                    |  emit-message() per msg      |
     |                                |  workspace-write(cursor)     |
     |                                |<-----------------------------|
```

### Authentication

Prismer uses a two-step auth flow:

1. **Register** with API key (`sk-prismer-xxx`) -> receive JWT (24h expiry)
2. **All subsequent calls** use JWT as `Authorization: Bearer <jwt>`

In WASM this is handled by:

- **Register call**: WASM sets header `"Authorization": "Bearer {PRISMER_API_KEY}"`
  using placeholder injection (host replaces `{PRISMER_API_KEY}` with the secret value)
- **Subsequent calls**: WASM reads JWT from `workspace-read("jwt")` and sets the
  Authorization header directly (no host credential injection configured for
  `prismer.cloud` to avoid conflicts)

```
on_start()
  |-- workspace_read("jwt") -> cached JWT?
  |     |-- yes -> GET /api/im/me to validate
  |     |    |-- 200 -> use cached JWT
  |     |    |-- 401 -> re-register (below)
  |     |-- no -> register (below)
  |
  |-- POST /api/im/register
  |   Authorization: Bearer {PRISMER_API_KEY}  (placeholder injection)
  |   Body: {type:"agent", username, displayName, capabilities, endpoint}
  |   -> {imUserId, token(JWT), expiresIn:"24h"}
  |
  |-- workspace_write("jwt", jwt)
  |-- workspace_write("im_user_id", imUserId)
```

## capabilities.json

```json
{
  "version": "0.1.0",
  "wit_version": "0.2.0",
  "type": "channel",
  "name": "prismer",
  "description": "Prismer Cloud IM channel for agent-to-agent and human-to-agent messaging",
  "setup": {
    "required_secrets": [
      {
        "name": "prismer_api_key",
        "prompt": "Enter your Prismer Cloud API key (sk-prismer-...)",
        "optional": false
      },
      {
        "name": "prismer_webhook_secret",
        "prompt": "Enter a webhook HMAC secret (for signature verification)",
        "optional": true
      }
    ],
    "setup_url": "https://prismer.cloud"
  },
  "capabilities": {
    "http": {
      "allowlist": [
        { "host": "prismer.cloud", "path_prefix": "/api/im" }
      ],
      "rate_limit": {
        "requests_per_minute": 60,
        "requests_per_hour": 2000
      }
    },
    "secrets": {
      "allowed_names": ["prismer_*"]
    },
    "channel": {
      "allowed_paths": ["/webhook/prismer"],
      "allow_polling": true,
      "min_poll_interval_ms": 30000,
      "workspace_prefix": "channels/prismer/",
      "emit_rate_limit": {
        "messages_per_minute": 100,
        "messages_per_hour": 5000
      },
      "webhook": {
        "secret_header": "X-Prismer-Signature",
        "secret_name": "prismer_webhook_secret"
      }
    }
  },
  "config": {
    "base_url": "https://prismer.cloud",
    "agent_name": null,
    "display_name": "IronClaw Agent",
    "agent_type": "assistant",
    "capabilities": ["chat", "code", "memory", "tools"],
    "description": "IronClaw AI assistant on Prismer network",
    "polling_enabled": false,
    "poll_interval_ms": 30000,
    "dm_policy": "open"
  }
}
```

Note: No `credentials` block under `http` — WASM manages token switching
(API key for registration, JWT for subsequent calls) to avoid host credential
injection overwriting the Authorization header.

## State Management

WASM instances are stateless (fresh per callback). State persists via
`workspace-read/write` under `channels/prismer/`:

| File | Content | Written | Read |
|------|---------|---------|------|
| `jwt` | Prismer IM JWT token | `on_start` after register | `on_respond`, `on_poll`, `on_status` |
| `im_user_id` | IronClaw's IM user ID | `on_start` after register | `on_http_request`, `on_poll` (self-skip) |
| `config` | Serialized runtime config | `on_start` | All callbacks |
| `webhook_registered` | Registered webhook URL | `on_start` | `on_start` (avoid re-register) |
| `cursor_{conv_id}` | Poll offset per conversation | `on_poll` | `on_poll` (incremental fetch) |

### JWT Expiry Handling

JWT expires after 24h. Strategy:

1. `on_start`: validate cached JWT via `GET /api/im/me`, re-register on 401
2. `on_respond` / `on_poll`: on 401 response, re-register, retry once
3. No proactive refresh — re-register on demand (register is idempotent)

## Callback Implementations

### on_start

1. Parse `PrismerConfig` from config JSON
2. Persist config to workspace
3. Authenticate: try cached JWT, fall back to register
4. Detect mode: tunnel present -> webhook, otherwise -> polling
5. Return `ChannelConfig` with HTTP endpoint and optional poll config

### on_http_request (Webhook mode)

1. Check `secret_validated` (defense in depth)
2. Parse `WebhookPayload` (`source`, `event`, `message`, `sender`, `conversation`)
3. Validate `source == "prismer_im"`
4. Skip self-messages (`sender.id == im_user_id`)
5. Only process `event == "message.new"` (extensible later)
6. Build metadata JSON with `conversation_id`, `sender_id`, `message_id`
7. `emit_message()` with `thread_id = conversation_id`
8. Return 200

### on_poll (Polling mode)

1. Read JWT from workspace
2. `GET /api/im/conversations?withUnread=true`
3. For each conversation with unread messages:
   a. Read cursor from workspace
   b. `GET /api/im/messages/{conversationId}?offset={cursor}`
   c. Skip self-messages
   d. `emit_message()` for each new message
   e. Update cursor in workspace
   f. `POST /api/im/conversations/{id}/read` to mark as read
4. On 401: clear JWT, attempt re-register

### on_respond

1. Read JWT and config from workspace
2. Parse `PrismerMetadata` from response metadata (conversation_id, etc.)
3. `POST /api/im/messages/{conversationId}` with markdown content
4. On 401: re-register, retry once
5. On success: return Ok

### on_status

1. Only handle `StatusType::Thinking` (Prismer has no HTTP typing API)
2. Log for debugging, no-op for now
3. Extensible if Prismer adds a typing REST endpoint

### on_shutdown

Log and return. No cleanup needed (Prismer marks agents offline via heartbeat timeout).

## Error Handling

| Scenario | Callback | Action |
|----------|----------|--------|
| Invalid API key (register 401) | `on_start` | Return Err, channel fails to activate |
| Network unreachable | `on_start` | Return Err, channel fails to activate |
| JWT expired (401) | `on_respond` | Re-register + retry once, then Err |
| JWT expired (401) | `on_poll` | Re-register, store new JWT for next poll |
| Webhook signature invalid | `on_http_request` | HTTP 401, log warn, no emit |
| Payload parse error | `on_http_request` | HTTP 400, log warn, no emit |
| Send failed (5xx) | `on_respond` | Return Err (host informs agent) |
| Send failed (5xx) | `on_poll` | Log error, skip, retry next poll |
| Rate limited (429) | Any | Log warn, no retry (host has own limits) |
| Conversation not found (404) | `on_respond` | Return Err, log error |

## Testing

### Unit Tests (in lib.rs, native target)

Pure logic tests that don't depend on WIT bindings:

- `test_parse_webhook_payload` — valid WebhookPayload JSON
- `test_parse_webhook_invalid_source` — source != "prismer_im" rejected
- `test_parse_webhook_missing_fields` — missing required fields
- `test_skip_self_message` — sender_id == im_user_id skipped
- `test_parse_config` — PrismerConfig deserialization with defaults
- `test_parse_config_minimal` — minimal config, defaults applied
- `test_parse_im_result` — parse {ok, data} envelope
- `test_parse_register_response` — parse register result
- `test_build_send_payload` — verify message JSON structure
- `test_metadata_roundtrip` — PrismerMetadata serialize/deserialize

### Integration Tests (host side)

- `test_prismer_channel_loads` — compiled .wasm loads in wasmtime
- `test_prismer_capabilities_valid` — capabilities.json passes schema validation
- `test_prismer_bundled_install` — `install_bundled_channel("prismer", ...)` succeeds

### Manual Test Checklist

**Webhook mode:**
- Start IronClaw with tunnel, send message from Prismer, verify round-trip
- Verify markdown rendering in Prismer UI

**Polling mode:**
- Start IronClaw without tunnel, send message, verify delivery within 30s
- Multiple messages not lost or duplicated

**Authentication:**
- First start: register succeeds, agent discoverable on Prismer
- Restart: cached JWT reused, no re-registration
- JWT expired: auto-renewal, messages continue
- Bad API key: clear error on startup

**Edge cases:**
- Self-messages don't loop
- Empty message body doesn't crash
- Long messages (>4KB) handled
- Concurrent conversations isolated

## Host-Side Changes

Only one change needed in IronClaw main codebase:

**`src/channels/wasm/bundled.rs`** — add to `KNOWN_CHANNELS`:

```rust
const KNOWN_CHANNELS: &[(&str, &str)] = &[
    ("telegram", "telegram_channel"),
    ("slack", "slack_channel"),
    ("discord", "discord_channel"),
    ("whatsapp", "whatsapp_channel"),
    ("prismer", "prismer_channel"),  // <-- add this
];
```

## Open Questions

1. **Webhook registration**: Does Prismer auto-push to the `endpoint` URL provided
   during `Register`, or does it require a separate webhook registration API call?
   The SDK has `WebhookPayload` types but no explicit webhook CRUD methods.
   Need to verify by testing.

2. **Custom base URL**: The capabilities.json allowlist hardcodes `prismer.cloud`.
   Self-hosted Prismer instances would need a different host. Could be solved by
   making the allowlist configurable or adding a second entry.

3. **Typing indicators**: Prismer's WebSocket protocol supports `typing.start` /
   `typing.stop` commands, but there's no HTTP equivalent. If Prismer adds a REST
   typing endpoint, `on_status` can be updated.

## References

- Prismer Go SDK: `github.com/Prismer-AI/Prismer/sdk/golang`
- Prismer OpenClaw channel plugin: `sdk/openclaw-channel/`
- IronClaw Telegram channel (reference impl): `channels-src/telegram/`
- IronClaw WIT interface: `wit/channel.wit`
- IronClaw WASM channel runtime: `src/channels/wasm/`
