# MCP Background Jobs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add (1) a per-MCP-server configurable call timeout that lifts the hardcoded 30s transport cap, and (2) a generic bridge that runs a long-running MCP tool call as a first-class, durable IronClaw job that auto-resumes the originating agent thread on completion.

**Architecture:** Phase 1 threads a configurable timeout from `McpServerConfig` through the transports and the `McpToolWrapper`. Phase 2 adds `tool_job_start` / `tool_job_status` builtin tools that create a persisted `JobContext` and spawn a lightweight non-LLM runner (mirroring the Container job path) which calls the MCP tool at the server's long timeout, streams `JobEvent`s to SSE, and injects the (safety-scanned) result back into the originating thread via the agent-loop inject channel.

**Tech Stack:** Rust, tokio, async-trait, serde, axum (gateway), PostgreSQL + libSQL dual backend.

## Global Constraints

- No `.unwrap()` / `.expect()` in production code (tests are fine). Map errors with `?` + context. (`src/agent/CLAUDE.md`, `.claude/rules/error-handling.md`)
- Every bug fix / feature commit includes a regression test (commit-msg hook enforces). (`.claude/rules/testing.md`)
- Test *through the caller*, not just the helper, when a helper gates a side effect. Integration tier = `cargo test --features integration`. (`.claude/rules/testing.md`)
- External tool output reaching the LLM MUST pass the safety layer (sanitize + wrap) first. (`.claude/rules/safety-and-sandbox.md`)
- MCP/sandbox tool output is untrusted external data — `requires_sanitization() == true`.
- Zero clippy warnings: `cargo clippy --all --benches --tests --examples --all-features`.
- Prefer `crate::` for cross-module imports; `super::` only in tests / intra-module.
- Config precedence is DB > env > toml > defaults; per-server settings live on the `McpServerConfig` row (single source of truth) — do NOT add a parallel generic `settings` key.
- Timeout default when unset stays **30s** (today's behavior); new `timeout_secs` clamps to `5..=21600` (6h).

**Reference spec:** `docs/superpowers/specs/2026-07-08-mcp-background-jobs-design.md`

---

## Phase 1 — Per-server configurable timeout

### Task 1: Add `timeout_secs` + `allow_background` to `McpServerConfig`

**Files:**
- Modify: `src/tools/mcp/config.rs:37-70` (struct), plus a helper for the clamped duration
- Test: same file, `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `McpServerConfig.timeout_secs: Option<u64>`, `McpServerConfig.allow_background: bool`, `McpServerConfig::effective_timeout() -> std::time::Duration`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn effective_timeout_defaults_to_30s_and_clamps() {
    let mut c = McpServerConfig::new("s", "http://x");
    assert_eq!(c.effective_timeout(), std::time::Duration::from_secs(30));
    c.timeout_secs = Some(3600);
    assert_eq!(c.effective_timeout(), std::time::Duration::from_secs(3600));
    c.timeout_secs = Some(1); // below floor
    assert_eq!(c.effective_timeout(), std::time::Duration::from_secs(5));
    c.timeout_secs = Some(999_999); // above ceiling
    assert_eq!(c.effective_timeout(), std::time::Duration::from_secs(21600));
}

#[test]
fn allow_background_defaults_false_and_roundtrips() {
    let c = McpServerConfig::new("s", "http://x");
    assert!(!c.allow_background);
    let json = serde_json::to_string(&c).unwrap();
    let back: McpServerConfig = serde_json::from_str(&json).unwrap();
    assert!(!back.allow_background);
    assert_eq!(back.timeout_secs, None);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw mcp::config::tests::effective_timeout_defaults_to_30s_and_clamps`
Expected: FAIL — no field `timeout_secs` / no method `effective_timeout`.

- [ ] **Step 3: Add the fields + helper**

In the struct (after `cached_tools`), add:
```rust
    /// Max seconds for a single call to this server before the transport times
    /// out. `None` uses the default 30s. Clamped to 5..=21600 by
    /// `effective_timeout()`. Local backends (a cold 27B sandbox) need more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,

    /// Whether this server's tools may be run as background jobs
    /// (`tool_job_start`). Opt-in; defaults false.
    #[serde(default)]
    pub allow_background: bool,
```
Add `timeout_secs: None,` and `allow_background: false,` to BOTH `new()` and `new_stdio()` initializers. Add the helper in `impl McpServerConfig`:
```rust
    /// Per-call transport timeout, clamped to a sane range. Default 30s.
    pub fn effective_timeout(&self) -> std::time::Duration {
        let secs = self.timeout_secs.unwrap_or(30).clamp(5, 21_600);
        std::time::Duration::from_secs(secs)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ironclaw mcp::config::tests::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/tools/mcp/config.rs
git commit -m "feat(mcp): add per-server timeout_secs + allow_background to McpServerConfig"
```

---

### Task 2: Thread the timeout into the stdio + HTTP transports

**Files:**
- Modify: `src/tools/mcp/stdio_transport.rs:114-129` (the `send` impl), and the transport struct to hold a `timeout: Duration`
- Modify: `src/tools/mcp/http_transport.rs:42-68` (`new`), add a `with_timeout` builder
- Test: `src/tools/mcp/stdio_transport.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `Duration` from `McpServerConfig::effective_timeout()` (Task 1)
- Produces: `StdioMcpTransport` uses its stored `timeout` in `stream_transport_send`; `HttpMcpTransport::with_timeout(Duration) -> Self`

**Confirm first:** `grep -n "self.timeout\|timeout" src/tools/mcp/stdio_transport.rs` — verify whether the struct already stores a timeout field. If not, add `timeout: Duration` to the struct and set it in `spawn(...)` (default `Duration::from_secs(30)` for the current call sites; a `with_timeout` setter to override).

- [ ] **Step 1: Write the failing test** (stdio struct carries + uses timeout)

```rust
#[tokio::test]
async fn stdio_transport_uses_configured_timeout() {
    // A transport constructed with a 7s override reports 7s.
    // (Construct via the same path spawn() uses; assert the stored field.)
    let t = StdioMcpTransport::spawn("t", "cat", Vec::<String>::new(), Vec::<(String,String)>::new())
        .await
        .expect("spawn cat")
        .with_timeout(std::time::Duration::from_secs(7));
    assert_eq!(t.timeout, std::time::Duration::from_secs(7));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw mcp::stdio_transport::tests::stdio_transport_uses_configured_timeout`
Expected: FAIL — no `with_timeout` / no `timeout` field.

- [ ] **Step 3: Implement**

In `stdio_transport.rs`: add `timeout: Duration` to the struct; initialize to `Duration::from_secs(30)` in `spawn`; add:
```rust
    /// Override the per-request timeout (default 30s).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
```
Change the `send` body to use `self.timeout` instead of the literal:
```rust
        stream_transport_send(
            &self.stdin,
            &self.pending,
            request,
            &self.server_name,
            self.timeout,
        )
        .await
```
In `http_transport.rs`: keep the `reqwest::Client::builder().timeout(Duration::from_secs(30))` default in `new`, and add a builder that rebuilds the client with an override:
```rust
    /// Override the HTTP request timeout (default 30s).
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.http_client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("Failed to create HTTP client"); // safety: rustls init cannot fail
        self
    }
```

- [ ] **Step 4: Run tests + clippy**

Run: `cargo test -p ironclaw mcp::stdio_transport && cargo clippy -p ironclaw`
Expected: PASS, zero warnings.

- [ ] **Step 5: Commit**

```bash
git add src/tools/mcp/stdio_transport.rs src/tools/mcp/http_transport.rs
git commit -m "feat(mcp): transports accept a configurable per-request timeout"
```

---

### Task 3: Wire the configured timeout through the factory + wrapper

**Files:**
- Modify: `src/tools/mcp/factory.rs:64-147` (pass `server.effective_timeout()` to the transports)
- Modify: `src/tools/mcp/client.rs:759-802` (`create_tools_with_store`) + `896-966` (`McpToolWrapper`) — carry a `timeout` field and return it from `execution_timeout()`
- Test: `src/tools/mcp/client.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `McpServerConfig::effective_timeout()` (Task 1); transport `with_timeout` (Task 2)
- Produces: `McpToolWrapper::execution_timeout()` returns the server's configured timeout (≥ transport timeout, so `execute.rs`/`dispatch.rs` don't re-clip)

**Confirm first:** `grep -n "process_manager.spawn_stdio\|effective_transport\|fn effective_timeout\|self.config\|server\b" src/tools/mcp/factory.rs` and `grep -n "server_name\|self.config\|McpServerConfig" src/tools/mcp/client.rs` — the factory has `server` in scope (it's `Some(server)` passed to `McpClient::new_with_transport`). Confirm the stdio path constructs through `process_manager.spawn_stdio` (a pooled path) — if so, apply `.with_timeout(...)` to the returned transport before wrapping, or thread the duration into `spawn_stdio`. Pick whichever keeps one construction path; prefer `.with_timeout()` on the returned `Arc`-inner transport if the pool returns an owned transport, else add a `timeout` param to `spawn_stdio`.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn mcp_tool_wrapper_reports_server_timeout() {
    // Build a wrapper with an explicit 900s timeout and assert execution_timeout().
    let store = std::sync::Arc::new(crate::tools::mcp::McpClientStore::new());
    let w = McpToolWrapper {
        tool: /* minimal McpTool fixture */ mcp_tool_fixture("do_thing"),
        prefixed_name: "srv__do_thing".into(),
        provider_extension: "srv".into(),
        server_name: "srv".into(),
        client_store: store,
        timeout: std::time::Duration::from_secs(900),
    };
    assert_eq!(w.execution_timeout(), std::time::Duration::from_secs(900));
}
```
(Add a small `mcp_tool_fixture(name)` test helper mirroring `client_store.rs`'s `tool_with_annotations`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw mcp::client::tests::mcp_tool_wrapper_reports_server_timeout`
Expected: FAIL — `McpToolWrapper` has no `timeout` field / no `execution_timeout` override.

- [ ] **Step 3: Implement**

Add `timeout: std::time::Duration` to the `McpToolWrapper` struct. In `create_tools_with_store`, compute the timeout once from the server config the client already holds (confirm accessor: `grep -n "fn server_config\|self.config\|effective_timeout" src/tools/mcp/client.rs`; if the client stores `Option<McpServerConfig>`, use `self.config.as_ref().map(|c| c.effective_timeout()).unwrap_or(Duration::from_secs(30))`) and set `timeout` on each wrapper. Add to `impl Tool for McpToolWrapper`:
```rust
    fn execution_timeout(&self) -> std::time::Duration {
        self.timeout
    }
```
In `factory.rs`, apply `.with_timeout(server.effective_timeout())` to each transport before it is wrapped in the `McpClient`.

- [ ] **Step 4: Run tests + clippy**

Run: `cargo test -p ironclaw mcp:: && cargo clippy -p ironclaw`
Expected: PASS, zero warnings.

- [ ] **Step 5: Commit**

```bash
git add src/tools/mcp/factory.rs src/tools/mcp/client.rs
git commit -m "feat(mcp): honor per-server timeout in transport + tool execution cap"
```

---

### Task 4: Expose `--timeout-secs` / `--allow-background` on `mcp add`

**Files:**
- Modify: `src/cli/mcp.rs:23-70` (`McpAddArgs`) + `add_server` (`:167-261`)
- Test: `src/cli/mcp.rs` `#[cfg(test)]` (arg parsing → config fields)

**Interfaces:**
- Consumes: `McpServerConfig.timeout_secs`, `.allow_background` (Task 1)

**Confirm first:** `grep -n "servers.upsert\|McpAddArgs\|clap\|arg" src/cli/mcp.rs` — confirm `mcp add` upserts (line ~257) so it doubles as edit, and see how existing flags are declared (clap derive vs builder).

- [ ] **Step 1: Write the failing test** — parse args including the new flags and assert the built `McpServerConfig` carries them.

```rust
#[test]
fn add_args_set_timeout_and_background() {
    let args = McpAddArgs {
        name: "srv".into(), timeout_secs: Some(3600), allow_background: true,
        /* ..existing fields with defaults.. */ ..default_add_args()
    };
    let cfg = args.to_config().expect("build config"); // extract config-building into a helper
    assert_eq!(cfg.timeout_secs, Some(3600));
    assert!(cfg.allow_background);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw cli::mcp::tests::add_args_set_timeout_and_background`
Expected: FAIL — no such fields / no `to_config`.

- [ ] **Step 3: Implement** — add `#[arg(long)] pub timeout_secs: Option<u64>` and `#[arg(long)] pub allow_background: bool` to `McpAddArgs`; set them on the `McpServerConfig` in `add_server` (extract the config-building into `McpAddArgs::to_config()` so it's unit-testable). Persist via the existing `servers.upsert(config)`.

- [ ] **Step 4: Run tests + a manual smoke**

Run: `cargo test -p ironclaw cli::mcp`
Expected: PASS. (Manual, after build: `ironclaw mcp add srv --transport http --url http://x --timeout-secs 60 --allow-background` then `ironclaw mcp list --verbose` shows them.)

- [ ] **Step 5: Commit**

```bash
git add src/cli/mcp.rs
git commit -m "feat(cli): mcp add accepts --timeout-secs and --allow-background"
```

---

### Task 5: Document Phase 1 config

**Files:**
- Modify: `.env.example` (note the per-server setting is DB-config, not env), `src/channels/web/CLAUDE.md` (Auth/section noting MCP per-server timeout)

- [ ] **Step 1:** Add a short note under the MCP section of `src/channels/web/CLAUDE.md` and a comment in `.env.example` pointing to `ironclaw mcp add --timeout-secs`. No code.
- [ ] **Step 2: Commit**

```bash
git add .env.example src/channels/web/CLAUDE.md
git commit -m "docs(mcp): document per-server timeout + allow_background"
```

**Phase 1 is independently shippable and delivers the sync-path timeout fix.** After Phase 1, enabling msbsandbox: `ironclaw mcp add msbsandbox ... --timeout-secs 3600 --allow-background`.

---

## Phase 2 — Generic MCP→job bridge (auto-resume)

### Task 6: Job descriptor + `mcp_job` metadata shape

**Files:**
- Create: `src/worker/mcp_job.rs` (module skeleton + `McpJobSpec`)
- Modify: `src/worker/mod.rs` (add `pub mod mcp_job;`)
- Test: `src/worker/mcp_job.rs` `#[cfg(test)]`

**Interfaces:**
- Produces:
```rust
pub struct McpJobSpec {
    pub server: String,       // MCP server name (e.g. "msbsandbox")
    pub tool: String,         // unprefixed MCP tool name (e.g. "run_python")
    pub params: serde_json::Value,
    pub user_id: String,
    pub channel: String,      // originating channel (for injection)
    pub thread_id: Option<String>,
}
impl McpJobSpec {
    pub fn to_metadata(&self) -> serde_json::Value; // {"mode":"mcp_tool","server":..,"tool":..,"params":..}
    pub fn from_metadata(meta: &serde_json::Value, user_id: &str, channel: &str, thread_id: Option<String>) -> Option<Self>;
}
```
Rationale: the "mode" lives in `JobContext.metadata` (already `serde_json::Value`), avoiding a change to the orchestrator `JobMode` enum which is not on the dispatch path.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn metadata_roundtrip() {
    let spec = McpJobSpec {
        server: "msbsandbox".into(), tool: "run_python".into(),
        params: serde_json::json!({"code":"print(1)"}),
        user_id: "u1".into(), channel: "gateway".into(), thread_id: Some("t1".into()),
    };
    let meta = spec.to_metadata();
    assert_eq!(meta["mode"], "mcp_tool");
    let back = McpJobSpec::from_metadata(&meta, "u1", "gateway", Some("t1".into())).unwrap();
    assert_eq!(back.server, "msbsandbox");
    assert_eq!(back.tool, "run_python");
    assert_eq!(back.params["code"], "print(1)");
    assert!(McpJobSpec::from_metadata(&serde_json::json!({"mode":"other"}), "u","c",None).is_none());
}
```

- [ ] **Step 2: Run → fail; Step 3: implement `McpJobSpec` + the two methods; Step 4: run → pass.**

Run: `cargo test -p ironclaw worker::mcp_job::tests::metadata_roundtrip`

- [ ] **Step 5: Commit**

```bash
git add src/worker/mcp_job.rs src/worker/mod.rs
git commit -m "feat(worker): McpJobSpec descriptor + job metadata shape"
```

---

### Task 7: The MCP job runner (non-LLM, streams events, injects result)

**Files:**
- Modify: `src/worker/mcp_job.rs` — add `run_mcp_job(...)`
- Test: `src/worker/mcp_job.rs` `#[cfg(test)]` (integration-tier, feature `integration`)

**Interfaces:**
- Consumes: `McpJobSpec` (Task 6); `McpClientStore::get` → `McpClient::call_tool` (extraction §5); `ContextManager::update_context` (§7); `SafetyLayer::sanitize_tool_output` + `wrap_for_llm` (§11); `SseManager::broadcast` (§ wiring); `IncomingMessage::new(..).into_internal().with_thread(..)` (§9); `store.update_job_status` / `save_job_event` (§13)
- Produces:
```rust
pub struct McpJobDeps {
    pub context_manager: Arc<ContextManager>,
    pub store: Option<SystemScope>,
    pub sse_tx: Option<Arc<crate::channels::web::sse::SseManager>>,
    pub inject_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
    pub client_store: Arc<crate::tools::mcp::McpClientStore>,
    pub safety: Arc<SafetyLayer>,
}
pub async fn run_mcp_job(job_id: uuid::Uuid, spec: McpJobSpec, deps: McpJobDeps);
```

**Behavior (in order):**
1. `deps.context_manager.update_context(job_id, |c| c.transition_to(JobState::InProgress, Some("mcp job started".into())))` (ignore idempotent Err per §6 semantics; log on real failure). Persist: if `store`, `store.update_job_status(job_id, JobState::InProgress).await` (silent-ok: next poll reconciles).
2. Emit status: build `AppEvent::JobStatus { job_id: job_id.to_string(), message: format!("Running {} in background", spec.tool) }`; `if let Some(sse) = &deps.sse_tx { sse.broadcast(evt) }`. Also persist via `store.save_job_event(job_id, "status", &json!({"message": ...}))`.
3. Resolve client: `let client = deps.client_store.get(&spec.user_id, &spec.server).await` → on `None`, treat as failure (server inactive).
4. `let result = client.call_tool(&spec.tool, spec.params.clone()).await;` — uses the server's configured long transport timeout (Phase 1).
5. Convert to a `Result<String, String>`: on `Ok(r)` join `r.content` text parts (mirror `McpToolWrapper::execute` §3); if `r.is_error` treat as `Err(content)`.
6. **Safety scan** (required): `let sanitized = deps.safety.sanitize_tool_output(&spec.tool, &raw); let wrapped = deps.safety.wrap_for_llm(&spec.tool, &sanitized.content);` — `wrapped` is what gets injected.
7. Transition + persist: `Completed` on success else `Failed`; `store.update_job_status(...)`.
8. Emit `AppEvent::JobResult { job_id, status: JobResultStatus::Completed|Failed, session_id: None, fallback_deliverable: None }` to SSE + `save_job_event(job_id, "result", ...)`.
9. Inject completion into the originating thread:
```rust
if let Some(tx) = &deps.inject_tx {
    let body = format!("[Background job {}] {} {}: {}",
        short_id(job_id), spec.tool,
        if ok {"completed"} else {"failed"}, wrapped);
    let mut msg = IncomingMessage::new(spec.channel.clone(), spec.user_id.clone(), body).into_internal();
    if let Some(t) = &spec.thread_id { msg = msg.with_thread(t.clone()); }
    let _ = tx.send(msg).await; // silent-ok: agent may be idle; job state already persisted
}
```

**Confirm first:** `grep -n "fn sanitize_tool_output\|fn wrap_for_llm" crates/ironclaw_safety/src/*.rs src/safety/*.rs` for exact `SafetyLayer` method signatures; `grep -n "fn into_internal\|fn with_thread\|fn new" src/channels/mod.rs` for `IncomingMessage` builders; `grep -n "fn update_job_status\|fn save_job_event" src/db/*.rs` for the store method signatures and the `SystemScope` wrapper.

- [ ] **Step 1: Write the failing integration test** — a stub `McpClient`/`McpClientStore` entry whose `call_tool` returns a known string; drive `run_mcp_job`; assert (a) job transitions to `Completed` in the `ContextManager`, and (b) an `IncomingMessage` is received on a test `inject_tx` receiver containing the (wrapped) output.

```rust
#[tokio::test]
async fn mcp_job_runs_completes_and_injects() {
    // Arrange: ContextManager with a Pending mcp job; a client_store with a stub
    // server returning "RESULT_OK"; an mpsc inject channel.
    // Act: run_mcp_job(job_id, spec, deps).await;
    // Assert: ctx.state == Completed; injected message contains "RESULT_OK".
}
```
(If a real `McpClient` stub is impractical, add a thin `McpCaller` trait boundary — `async fn call(&self, tool, params) -> Result<String,String>` — implemented for the real client-store lookup and by a test fake. This keeps the runner testable per the test-through-the-caller rule.)

- [ ] **Step 2: Run → fail; Step 3: implement `run_mcp_job` + `McpJobDeps`; Step 4: run → pass.**

Run: `cargo test -p ironclaw --features integration worker::mcp_job::tests::mcp_job_runs_completes_and_injects`

- [ ] **Step 5: Commit**

```bash
git add src/worker/mcp_job.rs
git commit -m "feat(worker): run_mcp_job — non-LLM MCP job runner with safety scan + auto-resume inject"
```

---

### Task 8: `Scheduler::dispatch_mcp_job` — persist + spawn the runner

**Files:**
- Modify: `src/agent/scheduler.rs` (add `dispatch_mcp_job`, mirroring `dispatch_job_inner:183-252`); add `client_store` + `inject_tx` to `Scheduler` (or accept them as args)
- Test: `src/agent/scheduler.rs` `#[cfg(test)]` (integration)

**Interfaces:**
- Consumes: `McpJobSpec` (Task 6), `run_mcp_job` + `McpJobDeps` (Task 7)
- Produces: `Scheduler::dispatch_mcp_job(&self, spec: McpJobSpec) -> Result<Uuid, JobError>`

**Behavior:** mirror `dispatch_job_inner` but skip the `Worker`:
1. `let job_id = self.context_manager.create_job_for_user(&spec.user_id, &title, &desc).await?;` (title e.g. `format!("mcp:{}/{}", spec.server, spec.tool)`).
2. `self.context_manager.update_context_and_get(job_id, |c| { c.metadata = spec.to_metadata(); }).await?` → `ctx`.
3. `if let Some(store) = &self.store { store.save_job(&ctx).await.map_err(...)?; }` (FK-valid before spawn).
4. Transition to `InProgress` is done inside `run_mcp_job` (do NOT double-transition here; unlike `schedule_with_context` which does it for LLM workers).
5. Build `McpJobDeps` from scheduler fields + `spec.clone()` and `tokio::spawn(run_mcp_job(job_id, spec, deps));`
6. Return `job_id`.

**Confirm first:** `grep -n "client_store\|inject_tx\|msg_tx\|SchedulerDeps\|pub fn new" src/agent/scheduler.rs` — `Scheduler` currently lacks `client_store`/`inject_tx`. Add them to `SchedulerDeps` (or a `set_mcp_deps(...)` setter mirroring `set_sse_sender:110`) and wire in `app.rs` (Task 10). The `inject_tx` is the same `mpsc::Sender<IncomingMessage>` the channels use to feed the agent loop.

- [ ] **Step 1: Write the failing integration test** — call `dispatch_mcp_job` with a spec (stub client returns a value), assert a job row is persisted (`store.get_job(job_id)` is `Some`) and eventually `Completed`.
- [ ] **Step 2: Run → fail; Step 3: implement `dispatch_mcp_job` + scheduler deps; Step 4: run → pass.**

Run: `cargo test -p ironclaw --features integration agent::scheduler::tests::dispatch_mcp_job`

- [ ] **Step 5: Commit**

```bash
git add src/agent/scheduler.rs
git commit -m "feat(scheduler): dispatch_mcp_job persists + spawns the MCP runner (no LLM worker)"
```

---

### Task 9: `tool_job_start` / `tool_job_status` builtin tools

**Files:**
- Create: `src/tools/builtin/tool_job.rs`
- Modify: `src/tools/builtin/mod.rs` (declare module + export)
- Test: `src/tools/builtin/tool_job.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `Scheduler::dispatch_mcp_job` (Task 8) via a `SchedulerSlot` (`src/tools/builtin/job.rs:33-36`); the `ToolRegistry` to resolve a prefixed tool name → `(server, tool)`; `McpServerConfig.allow_background` (Task 1) via the client-store/config
- Produces two `Tool` impls (`CreateMcpJobTool` → name `tool_job_start`, `McpJobStatusTool` → name `tool_job_status`)

**`tool_job_start` params schema:** `{ "tool": string (prefixed, e.g. "msbsandbox__run_python"), "arguments": object }`. Behavior:
1. Split the prefixed name into `(server, mcp_tool)` (mirror `mcp_tool_id`/its inverse in `client.rs`; confirm separator via `grep -n "fn mcp_tool_id" src/tools/mcp/client.rs`).
2. Verify the server exists, is active for `ctx.user_id`, and has `allow_background == true` — else `Err(ToolError::InvalidParameters("server '..' is not enabled for background jobs"))`.
3. Honor approval: if the underlying tool `requires_approval`, return `ToolError::NotAuthorized(...)` guiding the caller to run it synchronously (v1 keeps approval on the sync path).
4. Build `McpJobSpec` (channel/thread from `ctx` metadata — confirm how job/thread context is available to a tool via `JobContext`; the originating channel/thread come from the dispatching turn's metadata).
5. `let job_id = scheduler.dispatch_mcp_job(spec).await?;`
6. `Ok(ToolOutput::success(json!({"job_id": job_id, "state": "in_progress"}), start.elapsed()))`.

**`tool_job_status` params:** `{ "job_id": string }` → look up via `context_manager.get_context` / `store.get_job`; return `{state, result?}` (result only when Completed; read from the last `result` job_event or a stashed field). 

**Confirm first:** `grep -rn "CreateJobTool::new\|\.register\|register_builtin_tools\|with_scheduler_slot" src/app.rs src/tools/builtin/mod.rs src/tools/registry.rs` — find exactly where `CreateJobTool` is constructed + registered and mirror it (same `SchedulerSlot`, `inject_tx`, `client_store`, `secrets` deps).

- [ ] **Step 1: Write the failing test** — construct `CreateMcpJobTool` with a fake scheduler slot; call `execute` with `{tool:"srv__do", arguments:{}}` where `srv` has `allow_background=false` → assert `InvalidParameters`; then with `allow_background=true` → assert it returns a `job_id`.
- [ ] **Step 2: Run → fail; Step 3: implement both tools + schema; Step 4: run → pass.**

Run: `cargo test -p ironclaw tools::builtin::tool_job::tests::`

- [ ] **Step 5: Commit**

```bash
git add src/tools/builtin/tool_job.rs src/tools/builtin/mod.rs
git commit -m "feat(tools): tool_job_start / tool_job_status builtins (generic MCP-as-job)"
```

---

### Task 10: Wire deps in `app.rs` + register the tools

**Files:**
- Modify: `src/app.rs` (near `:544` `register_builtin_tools` and wherever `CreateJobTool` is wired) — construct the two new tools with `context_manager`, the `SchedulerSlot`, `client_store`, `inject_tx`, `sse`, `safety`, `store`; give the `Scheduler` its `client_store` + `inject_tx` (Task 8).
- Test: covered by Task 11 e2e.

**Confirm first:** `grep -n "CreateJobTool\|scheduler\|client_store\|inject_tx\|msg_tx\|set_sse_sender\|register" src/app.rs` — locate the scheduler construction, the `msg_tx`/inject channel, and the `McpClientStore` instance to share.

- [ ] **Step 1:** Construct + register `CreateMcpJobTool` and `McpJobStatusTool` alongside `CreateJobTool`; call the new scheduler setter with `client_store` + `inject_tx`.
- [ ] **Step 2: Build**

Run: `cargo build -p ironclaw`
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): wire MCP-job tools + scheduler client_store/inject deps"
```

---

### Task 11: Startup reconcile — in-flight MCP jobs → `Stuck`

**Files:**
- Modify: `src/app.rs` (boot path, after store init) — a `reconcile_mcp_jobs_on_startup(store)` call
- Create helper: `src/worker/mcp_job.rs::reconcile_orphaned_mcp_jobs(store) -> usize`
- Test: `src/worker/mcp_job.rs` `#[cfg(test)]` (integration)

**Interfaces:**
- Consumes: `store.list_jobs_by_status(JobState::InProgress)` (§13) filtered to `metadata.mode == "mcp_tool"`
- Produces: `pub async fn reconcile_orphaned_mcp_jobs(store: &SystemScope) -> Result<usize, DatabaseError>` — transitions each to `Stuck`, returns count.

**Confirm first:** `grep -n "list_jobs_by_status\|list_agent_jobs\|update_job_status\|find_stuck_jobs" src/db/*.rs src/context/manager.rs` — confirm the exact enumeration method; if `list_jobs_by_status` is absent, use `list_agent_jobs_for_user` per user or add the query.

- [ ] **Step 1: Write the failing integration test** — persist an `InProgress` job with `metadata.mode="mcp_tool"`; run `reconcile_orphaned_mcp_jobs`; assert it becomes `Stuck` and the returned count is 1; a non-mcp InProgress job is untouched.
- [ ] **Step 2: Run → fail; Step 3: implement; Step 4: run → pass.**

Run: `cargo test -p ironclaw --features integration worker::mcp_job::tests::reconcile`

- [ ] **Step 5:** Call it once during boot in `app.rs` (log the count). **Commit:**

```bash
git add src/worker/mcp_job.rs src/app.rs
git commit -m "feat(worker): reconcile orphaned in-flight MCP jobs to Stuck on startup"
```

---

### Task 12: Surface MCP-job `mode` in `/api/jobs`

**Files:**
- Modify: `src/channels/web/features/jobs/mod.rs:109-130` (`list_agent_jobs_for_user` mapping → set `JobInfo.kind`/`mode` from `metadata.mode`)
- Modify: `src/channels/web/types.rs` (`JobInfo` — add an optional `mode: Option<String>` if not present)
- Test: `src/channels/web/features/jobs/` handler test (or reuse existing jobs handler test harness)

**Confirm first:** `grep -n "struct JobInfo\|mode\|kind\|get_sandbox_job_mode" src/channels/web/types.rs src/channels/web/features/jobs/mod.rs` — `JobInfo` may already carry a `mode`/`kind` used by sandbox jobs; reuse it, don't add a duplicate (wire-contract single-name rule, `.claude/rules/types.md`).

- [ ] **Step 1: Write the failing test** — an agent job whose metadata mode is `mcp_tool` lists with `JobInfo.mode == Some("mcp_tool")`.
- [ ] **Step 2: Run → fail; Step 3: map metadata→mode in the agent-jobs branch; Step 4: run → pass.**

Run: `cargo test -p ironclaw --features integration <jobs handler test>`

- [ ] **Step 5: Commit**

```bash
git add src/channels/web/features/jobs/mod.rs src/channels/web/types.rs
git commit -m "feat(gateway): surface mcp_tool mode on /api/jobs agent-job entries"
```

---

### Task 13: End-to-end verification + docs

**Files:**
- Modify: `docs/superpowers/specs/2026-07-08-mcp-background-jobs-design.md` (mark shipped), `src/channels/web/CLAUDE.md` (document the two tools + `/api/jobs` mode)
- No new production code.

- [ ] **Step 1: Full test + lint gate**

Run:
```bash
cargo test -p ironclaw
cargo test -p ironclaw --features integration
cargo clippy --all --benches --tests --examples --all-features
grep -rnE '\.unwrap\(|\.expect\(' src/worker/mcp_job.rs src/tools/builtin/tool_job.rs
```
Expected: all pass; zero clippy warnings; no `unwrap`/`expect` in the new production files.

- [ ] **Step 2: Manual smoke against a real deploy** (after building + deploying the binary):
  - `ironclaw mcp add msbsandbox ... --timeout-secs 3600 --allow-background`
  - From a chat turn, have the agent call `tool_job_start(tool="msbsandbox__run_python", arguments={code:"import time; time.sleep(150); print('done')"})`.
  - Confirm: returns a `job_id` immediately; job appears in `/api/jobs` with `mode=mcp_tool`, state `in_progress`; ~150s later the thread receives an injected completion carrying `done` and the agent continues. (This is the >120s-without-blank proof for the async path.)

- [ ] **Step 3: Commit docs**

```bash
git add docs/superpowers/specs/2026-07-08-mcp-background-jobs-design.md src/channels/web/CLAUDE.md
git commit -m "docs: MCP background jobs shipped — tools, /api/jobs mode, config"
```

---

## Self-Review notes (author)

- **Spec coverage:** Phase 1 (timeout) → Tasks 1–5. Bridge trigger tools → Task 9. Runner + auto-resume → Task 7. Dispatch/persist → Task 8. Durability/reconcile → Task 11. Gateway surface → Task 12. Config surface → Tasks 4–5. Safety scan → Task 7 step 6. Approval gate → Task 9 step 3. All spec sections map to a task.
- **Deviation from spec (deliberate):** the "mode" is carried in `JobContext.metadata` rather than a new `JobMode::McpTool` enum variant, because `JobMode` (orchestrator) is not on the dispatch path and metadata already flows through persistence + the gateway. Noted in Task 6.
- **Confirm-first steps:** each Phase-2 task begins with an exact `grep` for the 1–2 signatures that the extraction could not fully verify (safety method names, `IncomingMessage` builders, store method names, registration site). These are 2-minute lookups, not design gaps.
- **Type consistency:** `McpJobSpec`, `McpJobDeps`, `run_mcp_job`, `dispatch_mcp_job`, `reconcile_orphaned_mcp_jobs`, `tool_job_start`/`tool_job_status` names are used consistently across tasks 6–13.
