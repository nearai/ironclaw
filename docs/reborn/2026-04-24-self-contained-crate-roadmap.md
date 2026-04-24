# IronClaw Reborn — Self-Contained Crate Roadmap

**Status:** Draft for review — local only until merged  
**Date:** 2026-04-24  
**Related docs:**

- `docs/reborn/2026-04-24-os-like-architecture-design.md`
- `docs/reborn/2026-04-24-architecture-faq-decisions.md`

---

## 1. Purpose

Define the next implementation steps to make the OS-like IronClaw Reborn architecture real in a self-contained, crate-by-crate way.

The goal is not to rewrite IronClaw in one pass. The goal is to build a small vertical slice that proves the architecture:

```text
filesystem mount
  -> discover extension
  -> register capability
  -> dispatch process
  -> emit realtime event
  -> write durable state
```

If this slice is clean, later product behavior can move into first-party extensions without recreating the current brittle runtime.

---

## 2. Guiding implementation rule

Build lower-level contracts before product behavior.

Do not start with:

- CodeAct
- Monty
- full gateway rewrite
- all auth flows
- all filesystem backends
- self-repair
- GitHub extension
- hot reload

Start with the smallest host path that proves the OS-like model.

---

## 3. Recommended PR sequence

### PR 1 — Architecture docs

Already started on branch `reborn-architecture-docs`.

Includes:

- OS-like architecture design
- FAQ and decision log
- this roadmap

### PR 2 — `crates/ironclaw_filesystem`

Build the durable path/mount API.

### PR 3 — `crates/ironclaw_extensions`

Build manifest/discovery/capability declaration logic.

This may begin inside `ironclaw_kernel` if kept narrow, but a separate crate is preferred if implementation scope grows.

### PR 4 — `crates/ironclaw_processes`

Build dispatch/spawn/process table and a language-agnostic stdio process protocol.

### PR 5 — `crates/ironclaw_kernel`

Wire filesystem + extensions + processes into a composition-only host.

### PR 6 — `extensions/echo`

Add the first tiny extension proving the full path.

### PR 7 — `extensions/conversation`

Add normalized inbound routing, channel-to-thread mapping, and inbox/outbox semantics.

### PR 8 — `extensions/agent_loop_tools`

Move the default tool/capability agent loop into a first-party extension.

### PR 9 — `extensions/gateway`

Move gateway/channel behavior into a first-party extension and prove reconnect/cursor/outbox flow.

### PR 10 — auth/network/sandbox hardening

Add secret handles, mediated network, sandbox profile enforcement, and stronger scope propagation.

---

## 4. Milestone 0 — Freeze contracts in docs

Before coding each crate, write a short contract doc.

Suggested files:

```text
docs/reborn/contracts/filesystem.md
docs/reborn/contracts/extensions.md
docs/reborn/contracts/processes.md
docs/reborn/contracts/auth.md
docs/reborn/contracts/network.md
docs/reborn/contracts/events.md
docs/reborn/contracts/host.md
```

Each contract should include:

- owns
- does not own
- public API sketch
- dependency direction
- invariants
- minimum tests

This is the first guardrail against rebuilding the blob.

---

## 5. Milestone 1 — `ironclaw_filesystem`

### Purpose

Provide the durable path/mount surface that replaces the old Workspace abstraction.

### Crate

```text
crates/ironclaw_filesystem/
```

### Build

- `Filesystem` trait
- local filesystem backend
- in-memory backend for tests
- mount table
- scoped paths
- basic namespace layout

### V1 API sketch

```rust
trait Filesystem {
    async fn read(&self, path: &PathRef) -> Result<Bytes>;
    async fn write(&self, path: &PathRef, data: Bytes) -> Result<()>;
    async fn list(&self, path: &PathRef) -> Result<Vec<DirEntry>>;
    async fn stat(&self, path: &PathRef) -> Result<FileStat>;
}
```

Mount manager:

```rust
mount("/system/extensions", local_backend)
mount("/projects", local_backend)
```

### Tests

- cannot escape mounted root with `..`
- read/write roundtrip
- list/stat work
- mount routing works
- path normalization is deterministic
- missing mount returns a typed error

### Non-goals

Do not add:

- search/indexing
- transactions beyond backend-local atomic writes
- subscriptions/watch
- auth policy
- secret storage
- thread orchestration

---

## 6. Milestone 2 — `ironclaw_extensions`

### Purpose

Represent what can run.

### Crate

```text
crates/ironclaw_extensions/
```

### Build

- `ExtensionManifest`
- extension discovery from filesystem
- capability declarations
- trust class
- config/state/cache/bin folder validation

### Manifest sketch

```toml
id = "agent_loop_tools"
version = "0.1.0"
trust = "privileged"

[capabilities.handle_input]
description = "Run tool-based agent loop for a thread"
mode = "dispatch"
permission = "ask"

[paths]
config = "config/"
state = "state/"
cache = "cache/"
bin = "bin/"
```

### Tests

- valid manifest loads
- invalid manifest fails
- missing required fields fail
- missing folders are created or reported consistently
- capabilities extracted
- trust class parsed
- extension cannot declare invalid paths

### Non-goals

Do not add:

- process execution
- process table
- sandbox policy
- network policy
- auth policy
- agent loop behavior

---

## 7. Milestone 3 — `ironclaw_processes`

### Purpose

Represent what is running.

### Crate

```text
crates/ironclaw_processes/
```

### Build

- `ProcessId`
- `Process`
- process table
- `dispatch`
- `spawn`
- lifecycle states
- stdio JSON process protocol
- simple sandbox profile placeholder
- process events

### API sketch

```rust
async fn dispatch(
    capability: CapabilityRef,
    params: Value,
    scope: ExecutionScope,
) -> Result<Value>;

async fn spawn(
    capability: CapabilityRef,
    params: Value,
    scope: ExecutionScope,
) -> Result<ProcessId>;

async fn status(process_id: ProcessId) -> Result<ProcessStatus>;
async fn kill(process_id: ProcessId) -> Result<()>;
async fn subscribe(process_id: ProcessId) -> EventStream;
```

### V1 protocol

Use a language-agnostic stdio JSON protocol.

Messages:

- `handshake`
- `invoke`
- `invoke_result`
- `invoke_error`
- `cancel`
- `shutdown`
- `healthcheck`

### Tests

- dispatch calls simple extension and returns result
- spawn starts process and returns id
- status changes correctly
- kill works
- stdout/stderr captured as runtime events
- timeout kills process
- process cannot run outside allowed working directory

### Non-goals

Do not add:

- extension discovery
- extension manifest validation as source of truth
- thread persistence
- global auth policy
- global network policy
- routing/repair/reflection logic
- durable event storage

---

## 8. Milestone 4 — `ironclaw_kernel`

### Purpose

Compose the system.

### Crate

```text
crates/ironclaw_kernel/
```

### Build

- system builder
- filesystem + extension manager + process manager wiring
- event bus composition
- boot namespace
- extension registration into process manager

### API sketch

```rust
let kernel = KernelBuilder::new()
    .with_filesystem(fs)
    .with_extension_manager(extensions)
    .with_process_manager(processes)
    .build()
    .await?;
```

### Tests

- boot creates namespace
- discovers extension
- registers capabilities
- dispatches discovered capability
- emits realtime event
- writes durable event/audit record if configured

### Non-goals

Do not add:

- agent loop implementation
- gateway implementation
- TUI implementation
- product workflows
- repair logic
- direct GitHub/Slack/etc. behavior

---

## 9. Milestone 5 — first-party extensions

Only after the crate stack works.

### Start with `extensions/echo`

`echo` proves:

- manifest
- discovery
- capability registration
- dispatch
- process protocol
- config folder
- event flow

Demo:

```bash
ironclaw reborn dispatch echo.say '{"text":"hello"}'
```

Expected:

```json
{"text":"hello"}
```

### Then add `extensions/conversation`

Proves:

- normalized inbound schema
- channel-to-thread routing
- outbox paths
- configured agent-loop selection

### Then add `extensions/agent_loop_tools`

Proves:

- agent loop as extension
- thread state in filesystem
- step append
- capability dispatch

### Then add `extensions/gateway` and `extensions/tui`

Proves:

- channels as extensions
- reconnect/cursor/outbox model
- UI outside kernel

---

## 10. Milestone 6 — auth/network/sandbox hardening

Do not start here unless the team intentionally wants to prioritize security infrastructure before proving the execution path.

### `ironclaw_auth`

Start small:

- `SecretHandle`
- in-memory secret store for tests
- secret lease
- redaction helper
- local encrypted backend later

### `ironclaw_network`

Start small:

- mediated HTTP client
- allowlist policy
- audit event hook
- no raw extension network in hosted mode

### Sandbox hardening

Start with:

- profile type
- timeout
- working directory restriction
- environment allowlist
- output limit

Add stronger isolation later.

---

## 11. Minimum viable vertical slice

The first meaningful proof should include:

```text
crates/ironclaw_filesystem
crates/ironclaw_extensions
crates/ironclaw_processes
crates/ironclaw_kernel
extensions/echo
```

End-to-end flow:

```text
filesystem mount
  -> discover echo extension
  -> extract echo.say capability
  -> register capability with process manager
  -> dispatch echo.say
  -> run extension process
  -> emit runtime event
  -> return result
  -> write durable event/history if configured
```

This proves the architecture without product complexity.

---

## 12. Success criteria

The architecture is real when:

- `ironclaw_kernel` has almost no product logic
- `ironclaw_processes` does not discover extensions
- `ironclaw_extensions` does not run processes
- `ironclaw_filesystem` does not know about agents
- `agent_loop` can be deleted or replaced without touching kernel
- `gateway` can be deleted or replaced without touching kernel
- `echo` extension runs through the same path future extensions will use

---

## 13. Early architecture guardrails

Add guardrails as soon as the first crates exist:

- dependency checks between crates
- forbidden imports from extensions into kernel internals
- contract tests for manifests
- process protocol tests
- filesystem path traversal tests
- no outbound network bypasses in hosted mode
- no raw secret material in config fixtures

These tests are not polish. They are the mechanism that keeps the architecture from drifting.

---

## 14. Final recommendation

The next implementation work should be a sequence of small self-contained crates, not a broad product rewrite.

Start with the durable filesystem, then extension discovery, then process execution, then kernel composition, then a tiny `echo` extension.

Only after that path is working should the team move the agent loop, gateway, TUI, auth, network, sandboxing, GitHub, or self-repair into the new model.
