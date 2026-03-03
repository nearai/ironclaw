# StubChannel + Test Tier Separation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `StubChannel` test double to the shared test harness and separate tests into tiers so `cargo test` is fast and self-contained while integration tests requiring external services are explicitly gated.

**Architecture:** `StubChannel` goes in `src/testing.rs` alongside `StubLlm`, using an mpsc channel for message injection and `Arc<Mutex<Vec<...>>>` for capturing responses. Test tiers use the existing `integration` feature flag in Cargo.toml — tests needing external PostgreSQL move behind `#[cfg(feature = "integration")]` and lose the silent-skip `try_connect()` pattern.

**Tech Stack:** Rust, tokio mpsc, async-trait, existing `Channel` trait from `src/channels/channel.rs`

---

### Task 1: Build StubChannel

**Files:**
- Modify: `src/testing.rs` (add StubChannel after StubLlm, around line 190)

**Context:** The `Channel` trait (`src/channels/channel.rs:160-225`) requires 4 methods: `name()`, `start()`, `respond()`, `health_check()`. The remaining methods have default impls. `start()` returns a `MessageStream = Pin<Box<dyn Stream<Item = IncomingMessage> + Send>>`. The existing `StubLlm` in this file uses `AtomicU32` for call counting and `AtomicBool` for failure toggling — follow the same patterns.

**Step 1: Write the failing test**

Add at the bottom of the `mod tests` block in `src/testing.rs`:

```rust
#[tokio::test]
async fn test_stub_channel_inject_and_capture() {
    use futures::StreamExt;

    let (channel, sender) = StubChannel::new("test-channel");

    // Start the channel to get the message stream
    let mut stream = channel.start().await.expect("start failed");

    // Inject a message
    sender
        .send(IncomingMessage::new("test-channel", "user1", "hello"))
        .await
        .expect("send failed");

    // Read it from the stream
    let msg = stream.next().await.expect("stream ended");
    assert_eq!(msg.content, "hello");
    assert_eq!(msg.user_id, "user1");
    assert_eq!(msg.channel, "test-channel");

    // Send a response and verify it was captured
    let response = OutgoingResponse::text("world");
    channel.respond(&msg, response).await.expect("respond failed");

    let captured = channel.captured_responses();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].1.content, "world");
}

#[tokio::test]
async fn test_stub_channel_health_check() {
    let (channel, _sender) = StubChannel::new("healthy");
    channel.health_check().await.expect("health check failed");

    channel.set_healthy(false);
    assert!(channel.health_check().await.is_err());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib test_stub_channel -- --nocapture`
Expected: FAIL — `StubChannel` does not exist yet

**Step 3: Write the StubChannel implementation**

Add these imports at the top of `src/testing.rs` (merge with existing imports):

```rust
use std::sync::Mutex;
use tokio::sync::mpsc;
use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;
```

Add the struct and impl after `StubLlm` (before `TestHarness`):

```rust
/// A configurable channel stub for tests.
///
/// Supports:
/// - Message injection via the returned `mpsc::Sender`
/// - Response capture for assertion
/// - Status update capture
/// - Configurable health check failure
///
/// # Usage
///
/// ```rust,no_run
/// let (channel, sender) = StubChannel::new("test");
/// sender.send(IncomingMessage::new("test", "user1", "hello")).await.unwrap();
/// // ... run agent logic that calls channel.respond() ...
/// let responses = channel.captured_responses();
/// ```
pub struct StubChannel {
    name: String,
    rx: tokio::sync::Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    responses: Arc<Mutex<Vec<(IncomingMessage, OutgoingResponse)>>>,
    statuses: Arc<Mutex<Vec<StatusUpdate>>>,
    healthy: AtomicBool,
}

impl StubChannel {
    /// Create a new stub channel and its message sender.
    ///
    /// The sender is used by tests to inject messages into the channel's stream.
    /// The channel captures all responses and status updates for later assertion.
    pub fn new(name: impl Into<String>) -> (Self, mpsc::Sender<IncomingMessage>) {
        let (tx, rx) = mpsc::channel(64);
        let channel = Self {
            name: name.into(),
            rx: tokio::sync::Mutex::new(Some(rx)),
            responses: Arc::new(Mutex::new(Vec::new())),
            statuses: Arc::new(Mutex::new(Vec::new())),
            healthy: AtomicBool::new(true),
        };
        (channel, tx)
    }

    /// Get all captured (message, response) pairs.
    pub fn captured_responses(&self) -> Vec<(IncomingMessage, OutgoingResponse)> {
        self.responses.lock().expect("poisoned").clone()
    }

    /// Get all captured status updates.
    pub fn captured_statuses(&self) -> Vec<StatusUpdate> {
        self.statuses.lock().expect("poisoned").clone()
    }

    /// Set whether `health_check()` succeeds or fails.
    pub fn set_healthy(&self, healthy: bool) {
        self.healthy.store(healthy, Ordering::Relaxed);
    }
}

#[async_trait]
impl Channel for StubChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let rx = self
            .rx
            .lock()
            .await
            .take()
            .ok_or_else(|| ChannelError::StartupFailed {
                name: self.name.clone(),
                reason: "start() already called".to_string(),
            })?;
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.responses
            .lock()
            .expect("poisoned")
            .push((msg.clone(), response));
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        self.statuses.lock().expect("poisoned").push(status);
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        if self.healthy.load(Ordering::Relaxed) {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: self.name.clone(),
                reason: "stub set to unhealthy".to_string(),
            })
        }
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib test_stub_channel -- --nocapture`
Expected: Both `test_stub_channel_inject_and_capture` and `test_stub_channel_health_check` PASS

**Step 5: Commit**

```bash
git add src/testing.rs
git commit -m "feat(testing): add StubChannel test double for Channel trait"
```

---

### Task 2: Wire StubChannel into TestHarnessBuilder

**Files:**
- Modify: `src/testing.rs` — add channel field to `TestHarness` and builder method

**Context:** `TestHarnessBuilder` (line 213-303) builds `AgentDeps` with sensible defaults. We need to expose a `StubChannel` from the harness so tests can inject messages and inspect responses. The `ChannelManager` lives in `src/channels/manager.rs` — it takes `Box<dyn Channel>` via `add()`. The harness should optionally include a `ChannelManager` with a `StubChannel` pre-registered.

**Step 1: Write the failing test**

Add to `mod tests` in `src/testing.rs`:

```rust
#[cfg(feature = "libsql")]
#[tokio::test]
async fn test_harness_with_channel() {
    let harness = TestHarnessBuilder::new().with_stub_channel().build().await;

    let (sender, channel_manager) = harness
        .channel
        .as_ref()
        .expect("channel should be present");

    // Inject a message via sender
    sender
        .send(IncomingMessage::new("stub", "user1", "test message"))
        .await
        .expect("send failed");

    // Verify channel is registered in the manager
    let names = channel_manager.channel_names().await;
    assert!(names.contains(&"stub".to_string()));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib test_harness_with_channel -- --nocapture`
Expected: FAIL — `with_stub_channel()` does not exist

**Step 3: Implement the builder extension**

Add a `channel` field to `TestHarness`:

```rust
pub struct TestHarness {
    pub deps: AgentDeps,
    pub db: Arc<dyn Database>,
    /// Stub channel sender + manager, present if `with_stub_channel()` was called.
    pub channel: Option<(mpsc::Sender<IncomingMessage>, ChannelManager)>,
    #[cfg(feature = "libsql")]
    _temp_dir: tempfile::TempDir,
}
```

Add to `TestHarnessBuilder`:

```rust
pub struct TestHarnessBuilder {
    db: Option<Arc<dyn Database>>,
    llm: Option<Arc<dyn LlmProvider>>,
    tools: Option<Arc<ToolRegistry>>,
    stub_channel: bool,
}
```

Update `TestHarnessBuilder::new()` to include `stub_channel: false`.

Add method:

```rust
/// Include a StubChannel and ChannelManager in the harness.
pub fn with_stub_channel(mut self) -> Self {
    self.stub_channel = true;
    self
}
```

In the `build()` method, after constructing `deps`, add:

```rust
let channel = if self.stub_channel {
    let (stub, sender) = StubChannel::new("stub");
    let manager = ChannelManager::new();
    manager.add(Box::new(stub)).await;
    Some((sender, manager))
} else {
    None
};
```

Add the needed import: `use crate::channels::ChannelManager;`

Update the `TestHarness` construction to include `channel`.

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib test_harness_with_channel -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add src/testing.rs
git commit -m "feat(testing): wire StubChannel into TestHarnessBuilder"
```

---

### Task 3: Write ChannelManager unit test using StubChannel

**Files:**
- Modify: `src/channels/manager.rs` — add a `mod tests` block at the bottom

**Context:** `ChannelManager` (`src/channels/manager.rs`) has zero tests. It handles `add()`, `start_all()`, `respond()`, `broadcast()`, `health_check_all()`, `hot_add()`, and stream merging. The `StubChannel` enables us to test all of this without real channels. Currently the file has no `mod tests` block at all.

**Step 1: Write the failing tests**

Add at the bottom of `src/channels/manager.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::IncomingMessage;
    use crate::testing::StubChannel;
    use futures::StreamExt;

    #[tokio::test]
    async fn test_add_and_start_all() {
        let manager = ChannelManager::new();
        let (stub, sender) = StubChannel::new("test");

        manager.add(Box::new(stub)).await;

        let mut stream = manager.start_all().await.expect("start_all failed");

        // Inject a message through the stub
        sender
            .send(IncomingMessage::new("test", "user1", "hello"))
            .await
            .expect("send failed");

        // Should appear in the merged stream
        let msg = stream.next().await.expect("stream ended");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.channel, "test");
    }

    #[tokio::test]
    async fn test_respond_routes_to_correct_channel() {
        let manager = ChannelManager::new();
        let (stub, _sender) = StubChannel::new("alpha");

        // Keep a reference for response inspection
        let responses = stub.captured_responses_handle();
        manager.add(Box::new(stub)).await;

        let msg = IncomingMessage::new("alpha", "user1", "request");
        manager
            .respond(&msg, OutgoingResponse::text("reply"))
            .await
            .expect("respond failed");

        // Verify the stub captured the response
        let captured = responses.lock().expect("poisoned");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].1.content, "reply");
    }

    #[tokio::test]
    async fn test_respond_unknown_channel_errors() {
        let manager = ChannelManager::new();
        let msg = IncomingMessage::new("nonexistent", "user1", "test");
        let result = manager.respond(&msg, OutgoingResponse::text("hi")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_health_check_all() {
        let manager = ChannelManager::new();
        let (stub1, _) = StubChannel::new("healthy");
        let (stub2, _) = StubChannel::new("sick");
        stub2.set_healthy(false);

        manager.add(Box::new(stub1)).await;
        manager.add(Box::new(stub2)).await;

        let results = manager.health_check_all().await;
        assert!(results["healthy"].is_ok());
        assert!(results["sick"].is_err());
    }

    #[tokio::test]
    async fn test_start_all_no_channels_errors() {
        let manager = ChannelManager::new();
        let result = manager.start_all().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_injection_channel_merges() {
        let manager = ChannelManager::new();
        let (stub, _sender) = StubChannel::new("real");
        manager.add(Box::new(stub)).await;

        let mut stream = manager.start_all().await.expect("start_all failed");

        // Use the injection channel (simulating background task)
        let inject_tx = manager.inject_sender();
        inject_tx
            .send(IncomingMessage::new("injected", "system", "background alert"))
            .await
            .expect("inject failed");

        let msg = stream.next().await.expect("stream ended");
        assert_eq!(msg.content, "background alert");
    }
}
```

**Note:** The `test_respond_routes_to_correct_channel` test needs a way to inspect responses after the channel has been moved into the manager via `add(Box::new(stub))`. Since `add()` takes ownership, we need a `captured_responses_handle()` method on `StubChannel` that returns a clone of the `Arc<Mutex<Vec<...>>>` before the channel is moved. Add this method to `StubChannel`:

```rust
/// Get a shared handle to the response capture list.
///
/// Call this *before* moving the channel into a `ChannelManager`,
/// since `add()` takes ownership.
pub fn captured_responses_handle(&self) -> Arc<Mutex<Vec<(IncomingMessage, OutgoingResponse)>>> {
    Arc::clone(&self.responses)
}

/// Get a shared handle to the status capture list.
pub fn captured_statuses_handle(&self) -> Arc<Mutex<Vec<StatusUpdate>>> {
    Arc::clone(&self.statuses)
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test channels::manager::tests -- --nocapture`
Expected: FAIL — tests reference `StubChannel` which needs the handle methods

**Step 3: Add the handle methods to StubChannel (in src/testing.rs)**

See the `captured_responses_handle()` and `captured_statuses_handle()` methods above.

**Step 4: Run tests to verify they pass**

Run: `cargo test channels::manager::tests -- --nocapture`
Expected: All 6 tests PASS

**Step 5: Commit**

```bash
git add src/testing.rs src/channels/manager.rs
git commit -m "test(channels): add ChannelManager unit tests using StubChannel"
```

---

### Task 4: Gate external-service tests behind `integration` feature

**Files:**
- Modify: `tests/workspace_integration.rs:1` — change feature gate
- Modify: `tests/workspace_integration.rs:25-34` — remove `try_connect` pattern

**Context:** Currently `workspace_integration.rs` uses `#![cfg(feature = "postgres")]` which is a *default* feature, so it always compiles. Then `try_connect()` silently skips tests when PostgreSQL is unavailable. This means `cargo test` always "passes" even when these tests didn't run. The `integration` feature flag already exists in Cargo.toml (line 194) but nothing uses it.

The `heartbeat_integration.rs` already uses `#[ignore]` which is the right pattern for tests needing a live LLM — leave it alone.

The `ws_gateway_integration.rs`, `openai_compat_integration.rs`, `provider_chaos.rs`, `wasm_channel_integration.rs`, `pairing_integration.rs`, `config_round_trip.rs`, and `tool_schema_validation.rs` are all self-contained (mock servers, temp dirs, no external services) — leave them ungated.

**Step 1: Change the feature gate**

Replace line 1 of `tests/workspace_integration.rs`:

```rust
// Before:
#![cfg(feature = "postgres")]

// After:
#![cfg(all(feature = "postgres", feature = "integration"))]
```

**Step 2: Remove try_connect and the silent-skip pattern**

Delete the `try_connect()` function (lines 25-34) and remove all `if try_connect(&pool).await.is_none() { return; }` guards from every test function. The tests should fail loudly if the DB is unreachable — that's the point of explicitly opting in with `--features integration`.

Also update `heartbeat_integration.rs` line 1:

```rust
// Before:
#![cfg(feature = "postgres")]

// After:
#![cfg(all(feature = "postgres", feature = "integration"))]
```

(The `#[ignore]` on the test function stays — it additionally requires a live LLM.)

**Step 3: Verify default `cargo test` skips integration tests**

Run: `cargo test 2>&1 | grep -c "workspace_integration\|heartbeat_integration"`
Expected: 0 (no integration tests compiled)

**Step 4: Verify `cargo test --features integration` includes them**

Run: `cargo test --features integration -- --list 2>&1 | grep workspace_integration`
Expected: Lists the workspace integration test names (they'll fail without a DB, which is correct)

**Step 5: Commit**

```bash
git add tests/workspace_integration.rs tests/heartbeat_integration.rs
git commit -m "test: gate external-service tests behind integration feature flag

Replace silent try_connect() skip pattern with explicit feature gating.
cargo test now runs only self-contained tests.
cargo test --features integration runs tests requiring PostgreSQL."
```

---

### Task 5: Document test tiers in CLAUDE.md

**Files:**
- Modify: `CLAUDE.md` — update the "Build & Test" section

**Step 1: Add test tier documentation**

In the `## Build & Test` section of `CLAUDE.md`, after the existing commands, add:

```markdown
### Test Tiers

| Tier | Command | What runs | External deps |
|------|---------|-----------|---------------|
| Unit | `cargo test` | All `mod tests` + self-contained integration tests | None |
| Integration | `cargo test --features integration` | + PostgreSQL-dependent tests | Running PostgreSQL |
| Live | `cargo test --features integration -- --ignored` | + LLM-dependent tests | PostgreSQL + LLM API keys |

**Rules:**
- Default `cargo test` must pass with zero external services
- Tests needing PostgreSQL use `#![cfg(all(feature = "postgres", feature = "integration"))]`
- Tests needing live LLM/API keys additionally use `#[ignore]`
- Never use `try_connect()` skip patterns — if the feature is enabled, fail loudly
```

**Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: document test tier separation (unit/integration/live)"
```

---

## Test Classification Reference

| Test File | Tier | Gate | Reason |
|-----------|------|------|--------|
| `src/**` inline `mod tests` | Unit | `#[cfg(test)]` (default) | Uses StubLlm, libsql temp DBs |
| `tests/config_round_trip.rs` | Unit | none | Tempdir only |
| `tests/tool_schema_validation.rs` | Unit | none | Pure logic |
| `tests/pairing_integration.rs` | Unit | none | Tempdir only |
| `tests/provider_chaos.rs` | Unit | none | Mock providers |
| `tests/wasm_channel_integration.rs` | Unit | none | Test runtime |
| `tests/ws_gateway_integration.rs` | Unit | none | Starts own server |
| `tests/openai_compat_integration.rs` | Unit | none | Mock LLM + own server |
| `tests/html_to_markdown.rs` | Unit | `html-to-markdown` feature | Pure logic |
| `tests/workspace_integration.rs` | Integration | `postgres` + `integration` | Needs PostgreSQL |
| `tests/heartbeat_integration.rs` | Live | `postgres` + `integration` + `#[ignore]` | Needs PostgreSQL + LLM |
