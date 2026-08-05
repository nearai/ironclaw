---
description: Scaffold a new SSE event end-to-end (Rust backend to web frontend)
allowed-tools: Read, Edit, Write, Glob, Grep, Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*)
argument-hint: <event_name> [description]
model: opus
---

> # ⚠ DO NOT FOLLOW THIS COMMAND — IT TARGETS A DELETED CODEBASE
>
> **Every file path in the steps below is gone.** This procedure scaffolds into
> the retired v1 gateway SSE path: `src/channels/` (Steps 1–3),
> `crates/ironclaw_gateway/static/` (Steps 4–5), and `src/agent/` / `src/worker/`
> (Step 6) — the final checklist names the same dead files. The
> `ironclaw_gateway` crate and the entire root `src/` monolith were deleted from
> the tree; none of these files exist and none should be created. Only Step 7
> (`cargo fmt` / `clippy`) still means anything. Following this command produces
> new files in a directory layout the build does not know about.
>
> **This command needs rewriting onto the `ironclaw_webui` streaming path** —
> the Reborn projection/SSE frame served by `crates/product/ironclaw_webui`, with the
> client side in `crates/product/ironclaw_webui/frontend/`, over the event-stream
> substrate (`ironclaw_event_log` → `ironclaw_event_projections` →
> `ironclaw_event_streams`). That rewrite has not been done: the correct
> Reborn procedure is **not** written down here, and this banner deliberately
> does not guess at it.
>
> Until then, for a new user-visible event start from the `reborn-feature`
> skill and `.claude/rules/gateway-events.md` (the live Reborn events and
> transport-projection rules), not from the steps below.
>
> *Identified in PR #6944 (WS11.3 guidance drift hotfixes), which found the
> paths dead but scoped the rewrite out — replacing a scaffold procedure is new
> guidance, not a drift fix.*

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

**File**: ~~`crates/ironclaw_gateway/static/js/core/sse.js`~~ — **deleted; do not create.** The Reborn client lives in `crates/product/ironclaw_webui/frontend/`.

In the `connectSSE()` function, add a new `eventSource.addEventListener()` for the snake_case event name. Parse the JSON data and call a handler function.

Create the handler function that updates the DOM. Put it in the split file that matches its surface — e.g. `js/core/onboarding.js` for auth/onboarding handlers, `js/surfaces/chat.js` for chat message handlers, `js/surfaces/jobs.js` for sandbox job events. Follow existing patterns:
- `showApproval(data)` for complex card-style UI
- `addMessage(role, content)` for simple text
- `setStatus(text, spinning)` for status bar updates

## Step 5: Add CSS if needed

**File**: ~~`crates/ironclaw_gateway/static/styles/`~~ — **deleted; do not create.** Reborn styling lives with the SPA under `crates/product/ironclaw_webui/frontend/`.

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
