---
description: Scaffold a new SSE event end-to-end (Rust backend to web frontend)
allowed-tools: Read, Edit, Write, Glob, Grep, Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*)
argument-hint: <event_name> [description]
model: opus
---

Add a new SSE event called `$ARGUMENTS` to the IronClaw web gateway. This involves changes across 5 files in a specific order. Follow each step exactly.

## Step 1: Add `StatusUpdate` variant

**File**: `src/channels/channel.rs`

Find the `StatusUpdate` enum and add a new variant. Use the event name in PascalCase. Include any fields the event needs as named fields (not a generic String).

Example for reference (existing variants):
```rust
pub enum StatusUpdate {
    Thinking(String),
    ToolStarted { name: String },
    ToolCompleted { name: String, success: bool },
    Status(String),
    ApprovalNeeded {
        request_id: String,
        tool_name: String,
        description: String,
        parameters: serde_json::Value,
    },
}
```

## Step 2: Map to `SseEvent` in web channel

**File**: `src/channels/web/mod.rs`

Find the `send_status` method in the `Channel` impl for `WebChannel`. Add a match arm for the new `StatusUpdate` variant that maps it to an `SseEvent`. The SSE event name should be snake_case.

Look at existing match arms for the pattern. The event data is serialized as JSON.

## Step 3: Add types if needed

**File**: `src/channels/web/types.rs`

If the event carries structured data beyond a simple string, add a serializable DTO struct here. Use `#[derive(Debug, Clone, Serialize, Deserialize)]`. Follow the existing patterns in the file.

## Step 4: Add frontend handler

**File**: `src/channels/web/static/app.js`

In the `connectSSE()` function, add a new `eventSource.addEventListener()` for the snake_case event name. Parse the JSON data and call a handler function.

Create the handler function that updates the DOM. Follow existing patterns:
- `showApproval(data)` for complex card-style UI
- `addMessage(role, content)` for simple text
- `setStatus(text, spinning)` for status bar updates

## Step 5: Add CSS if needed

**File**: `src/channels/web/static/style.css`

If the event needs custom UI (cards, badges, etc.), add styles. Follow the existing naming conventions (`.approval-card`, `.log-entry`, etc.).

## Step 6: Send the event from Rust

Identify where in the backend this event should be triggered. Common locations:
- `src/agent/agent_loop.rs` - During message processing or tool execution
- `src/worker/job.rs` - During job execution
- `src/agent/heartbeat.rs` - During periodic execution

Use the existing pattern:
```rust
let _ = self.channels.send_status(
    &message.channel,
    StatusUpdate::YourNewVariant { ... },
    &message.metadata,
).await;
```

## Step 7: Quality gate

Run `cargo fmt` and `cargo clippy --all --benches --tests --examples --all-features` to verify the changes compile cleanly.

## Checklist

Before finishing, verify:
- [ ] `StatusUpdate` variant added in `channel.rs`
- [ ] Match arm added in `web/mod.rs` `send_status`
- [ ] DTO added in `types.rs` (if needed)
- [ ] `addEventListener` added in `app.js`
- [ ] Handler function created in `app.js`
- [ ] CSS styles added (if needed)
- [ ] Event sent from appropriate backend location
- [ ] `cargo fmt` clean
- [ ] `cargo clippy` clean
- [ ] Non-web channels unaffected (they ignore unknown StatusUpdate variants)
