# SSE Event System — IronClaw Real-Time Broadcast

## Overview

IronClaw uses Server-Sent Events (SSE) to stream real-time updates from the agent to web gateway clients. This document describes the event protocol, broadcast architecture, and integration points.

## Architecture

```
Agent Loop (dispatcher.rs / worker/job.rs)
         │
         ▼
  StatusUpdate::ToolStarted/Completed/etc.
         │
         ▼
  Channel::send_status()  ──┐
         │                  │
         ▼                  │
  WebChannel               │ (other channels: CLI, Telegram, etc.)
         │                  │
         ▼                  │
  AppEvent conversion       │
         │                  │
         ▼                  │
  SseManager.broadcast() ───┘
         │
         ▼
  broadcast::channel(256)
         │
         ▼
  Connected SSE clients (browser tabs, WebSocket bridge)
```

## Event Types

All events are defined in `crates/ironclaw_common/src/event.rs` as the `AppEvent` enum.

### Agent Lifecycle Events

| Event | Description | Payload |
|-------|-------------|---------|
| `Thinking` | Agent is processing | `message: String`, `thread_id: Option<String>` |
| `Response` | Final text response | `content: String`, `thread_id: String` |
| `StreamChunk` | Streaming response chunk | `content: String`, `thread_id: Option<String>` |
| `Status` | General status update | `message: String`, `thread_id: Option<String>` |
| `Error` | Error occurred | `message: String`, `thread_id: Option<String>` |

### Tool Execution Events

| Event | Description | Payload |
|-------|-------------|---------|
| `ToolStarted` | Tool execution began | `name: String`, `thread_id: Option<String>` |
| `ToolCompleted` | Tool execution finished | `name: String`, `success: bool`, `error: Option<String>`, `parameters: Option<String>`, `thread_id: Option<String>` |
| `ToolResult` | Tool result preview | `name: String`, `preview: String`, `thread_id: Option<String>` |

### Job Events (Background Tasks)

| Event | Description | Payload |
|-------|-------------|---------|
| `JobStarted` | Background job started | `job_id: String`, `title: String`, `browse_url: String` |
| `JobMessage` | Job agent message | `job_id: String`, `role: String`, `content: String` |
| `JobToolUse` | Job using a tool | `job_id: String`, `tool_name: String`, `input: Value` |
| `JobToolResult` | Job tool result | `job_id: String`, `tool_name: String`, `output: String` |
| `JobStatus` | Job status update | `job_id: String`, `message: String` |
| `JobResult` | Job completed | `job_id: String`, `status: String`, `session_id: Option<String>`, `fallback_deliverable: Option<Value>` |
| `JobReasoning` | Job reasoning update | `job_id: String`, `narrative: String`, `decisions: Vec<ToolDecisionDto>` |

### Approval & Auth Events

| Event | Description | Payload |
|-------|-------------|---------|
| `ApprovalNeeded` | Tool requires approval | `request_id: String`, `tool_name: String`, `description: String`, `parameters: String`, `thread_id: Option<String>`, `allow_always: bool` |
| `AuthRequired` | Extension needs auth | `extension_name: String`, `instructions: Option<String>`, `auth_url: Option<String>`, `setup_url: Option<String>` |
| `AuthCompleted` | Auth flow finished | `extension_name: String`, `success: bool`, `message: String` |

### Enhanced Events

| Event | Description | Payload |
|-------|-------------|---------|
| `ImageGenerated` | AI image created | `data_url: String`, `path: Option<String>`, `thread_id: Option<String>` |
| `Suggestions` | Follow-up suggestions | `suggestions: Vec<String>`, `thread_id: Option<String>` |
| `TurnCost` | Token/cost breakdown | `input_tokens: u64`, `output_tokens: u64`, `cost_usd: String`, `thread_id: Option<String>` |
| `ReasoningUpdate` | Agent reasoning | `narrative: String`, `decisions: Vec<ToolDecisionDto>`, `thread_id: Option<String>` |
| `ExtensionStatus` | WASM extension status | `extension_name: String`, `status: String`, `message: Option<String>` |
| `Heartbeat` | System alive (global) | *(no payload)* |

## Scoping Rules

Events can be **scoped** to a specific user or **global**:

- **Scoped** (`user_id: Some(id)`): Delivered only to subscribers for that user
- **Global** (`user_id: None`): Delivered to ALL subscribers (e.g., `Heartbeat`)

The `SseManager` wraps events in `ScopedEvent`:

```rust
pub struct ScopedEvent {
    pub user_id: Option<String>,
    pub event: AppEvent,
}
```

### Multi-Tenant Behavior

In multi-user mode:
- Events with `user_id` in metadata are scoped to that user
- Events without `user_id` leak across users (by design for system events)
- Each user gets their own SSE stream filtered by `user_id`

In single-user mode:
- All events delivered to all subscribers (backwards compatible)

## Broadcast Flow

### 1. Agent Emits StatusUpdate

From `dispatcher.rs`:

```rust
let _ = self
    .agent
    .channels
    .send_status(
        &self.message.channel,
        StatusUpdate::ToolStarted {
            name: tc.name.clone(),
        },
        &self.message.metadata,
    )
    .await;
```

### 2. WebChannel Converts to AppEvent

From `src/channels/web/mod.rs`:

```rust
async fn send_status(
    &self,
    status: StatusUpdate,
    metadata: &serde_json::Value,
) -> Result<(), ChannelError> {
    let thread_id = metadata
        .get("thread_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    
    let event = match status {
        StatusUpdate::ToolStarted { name } => AppEvent::ToolStarted {
            name,
            thread_id: thread_id.clone(),
        },
        // ... other variants
    };
    
    // Scope to user if available
    if let Some(uid) = metadata.get("user_id").and_then(|v| v.as_str()) {
        self.state.sse.broadcast_for_user(uid, event);
    } else {
        self.state.sse.broadcast(event); // global
    }
    Ok(())
}
```

### 3. SseManager Broadcasts

From `src/channels/web/sse.rs`:

```rust
pub fn broadcast_for_user(&self, user_id: &str, event: AppEvent) {
    let _ = self.tx.send(ScopedEvent {
        user_id: Some(user_id.to_string()),
        event,
    });
}

pub fn broadcast(&self, event: AppEvent) {
    let _ = self.tx.send(ScopedEvent {
        user_id: None, // global
        event,
    });
}
```

### 4. Clients Receive via SSE Stream

From `src/channels/web/sse.rs`:

```rust
pub fn subscribe(
    &self,
    user_id: Option<String>,
) -> impl Stream<Item = Result<Event, Infallible>> + Send + 'static {
    let rx = self.tx.subscribe();
    let mut stream = BroadcastStream::new(rx);
    // Filter by user_id match...
}
```

## Job Event Flow

Jobs have a dedicated event flow via `JobDelegate::log_event()`:

```rust
fn log_event(&self, event_type: &str, data: serde_json::Value) {
    // 1. Persist to DB
    if let Some(store) = self.store() {
        tokio::spawn(async move {
            store.save_job_event(job_id, event_type, &data).await.ok();
        });
    }
    
    // 2. Broadcast SSE
    if let Some(ref sse) = self.deps.sse_tx {
        let event = match event_type {
            "message" => AppEvent::JobMessage { ... },
            "tool_use" => AppEvent::JobToolUse { ... },
            "tool_result" => AppEvent::JobToolResult { ... },
            "status" => AppEvent::JobStatus { ... },
            "result" => AppEvent::JobResult { ... },
            "reasoning" => AppEvent::JobReasoning { ... },
            _ => None,
        };
        if let Some(event) = event {
            sse.broadcast(event);
        }
    }
}
```

## Connection Management

### Limits

- `MAX_CONNECTIONS = 100` — prevents resource exhaustion
- Buffer size: 256 events — slow clients may miss events (acceptable for SSE with reconnect)

### Lifecycle

1. Client connects to `/api/sse` or `/api/events`
2. `SseManager.subscribe_raw()` increments connection counter
3. Client receives filtered stream based on `user_id`
4. On disconnect, counter decremented (via `Drop`)

## Testing

### Unit Tests

See `src/channels/web/sse.rs` tests:

```rust
#[tokio::test]
async fn test_broadcast_delivers_to_all_subscribers() {
    let manager = SseManager::new();
    let mut sub1 = manager.subscribe_raw(None);
    let mut sub2 = manager.subscribe_raw(None);
    
    manager.broadcast(AppEvent::Heartbeat);
    
    assert!(sub1.next().await.is_some());
    assert!(sub2.next().await.is_some());
}

#[tokio::test]
async fn test_broadcast_for_user_scopes_correctly() {
    let manager = SseManager::new();
    let mut alice = manager.subscribe_raw(Some("alice".into()));
    let mut bob = manager.subscribe_raw(Some("bob".into()));
    
    manager.broadcast_for_user("alice", AppEvent::Thinking { ... });
    
    assert!(alice.next().await.is_some());
    // Bob should NOT receive alice's event
    tokio::time::timeout(Duration::from_millis(100), bob.next())
        .await
        .expect_err("Bob should not receive scoped event");
}
```

### Integration Tests

TODO: Add integration tests that:
1. Start a real web gateway
2. Connect SSE client
3. Trigger agent action (tool exec, job)
4. Verify events received in correct order

## Troubleshooting

### Events Not Appearing in UI

1. Check `user_id` scoping — is the client subscribed with the right `user_id`?
2. Verify `metadata.thread_id` is present — some clients drop events without it
3. Check connection count — are you hitting `MAX_CONNECTIONS`?
4. Inspect `tracing` logs for broadcast failures

### Missing Events After Restart

SSE events are **not persisted** — they're real-time only. After server restart:
- Pending approvals are lost (in-memory only)
- Job events must be re-fetched from DB (`/api/jobs/:id/events`)
- Clients should reconnect and rehydrate state from API

### Multi-Tenant Leaks

If events are leaking across users:
1. Verify `metadata.user_id` is set in the channel message
2. Check `broadcast_for_user()` is called (not `broadcast()`)
3. Review `ScopedEvent` filtering in `subscribe_raw()`

## Future Enhancements

- [ ] Event persistence buffer (replay last N events on reconnect)
- [ ] Event compression for high-frequency streams
- [ ] WebSocket bidirectional support (currently SSE is unidirectional)
- [ ] Event filtering/subscriptions (clients subscribe to specific event types)
- [ ] Rate limiting per user/channel

## Related Files

- `crates/ironclaw_common/src/event.rs` — `AppEvent` enum definition
- `src/channels/web/sse.rs` — `SseManager` implementation
- `src/channels/web/mod.rs` — `WebChannel::send_status()` conversion
- `src/channels/channel.rs` — `StatusUpdate` enum
- `src/worker/job.rs` — `JobDelegate::log_event()` for job events
- `src/agent/dispatcher.rs` — Tool execution status emission
