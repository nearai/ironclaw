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
  -> discover extension manifest
  -> register capability
  -> execute WASM capability
  -> expose MCP capability
  -> run script runner helper
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
- arbitrary live in-flight hot migration

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

### PR 4 — `crates/ironclaw_wasm` + WASM echo

Build the default installed capability lane and prove one tiny WASM capability.

### PR 5 — `crates/ironclaw_kernel`

Wire filesystem + extensions + WASM runtime into a composition-only host.

### PR 6 — `crates/ironclaw_mcp`

Adapt existing MCP servers/tools into IronClaw capabilities.

### PR 7 — `crates/ironclaw_scripts`

Add `script.run` for project-local sandboxed Python/bash/JS helpers.

### PR 8 — `extensions/conversation` and `extensions/missions`

Add normalized inbound routing, channel-to-thread mapping, inbox/outbox semantics, and mission definition execution.

### PR 9 — `extensions/agent_loop_tools`

Move the default tool/capability agent loop into a first-party extension.

### PR 10 — `extensions/gateway` and `extensions/tui`

Move gateway/TUI channel behavior into first-party extensions and prove reconnect/cursor/outbox flow.

### PR 11 — auth/network/sandbox hardening

Add secret handles, mediated network, sandbox profile enforcement, and stronger scope propagation.

---

## 4. Milestone 0 — Freeze contracts in docs

Before coding each crate, write a short contract doc.

Suggested files:

```text
docs/reborn/contracts/filesystem.md
docs/reborn/contracts/extensions.md
docs/reborn/contracts/wasm.md
docs/reborn/contracts/mcp.md
docs/reborn/contracts/scripts.md
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
id = "echo"
version = "0.1.0"
trust = "sandboxed"

[runtime]
type = "wasm"
module = "bin/echo.wasm"

[capabilities.say]
description = "Echo text"
mode = "dispatch"
permission = "allow"

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

## 7. Milestone 3 — `ironclaw_wasm`

### Purpose

Provide the default installed capability lane.

### Crate

```text
crates/ironclaw_wasm/
```

### Build

- WASM module loader
- host ABI/import surface
- module validation
- fuel/time/memory/output limits
- capability invocation wrapper
- scoped imports for filesystem/auth/network/events/dispatch

### API sketch

```rust
async fn invoke_wasm(
    module: WasmModuleRef,
    capability: CapabilityRef,
    params: Value,
    scope: ExecutionScope,
) -> Result<Value>;
```

### Tests

- valid module loads
- invalid module fails
- capability export is invoked
- fuel/time limit stops runaway code
- memory/output limits are enforced
- filesystem/network/auth imports require scoped grants

### Non-goals

Do not add:

- MCP protocol handling
- project-local Python/bash/JS execution
- marketplace behavior
- product workflows

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
- filesystem + extension manager + WASM runtime wiring
- event bus composition
- boot namespace
- extension capability registration into host dispatch table

### API sketch

```rust
let kernel = KernelBuilder::new()
    .with_filesystem(fs)
    .with_extension_manager(extensions)
    .with_wasm_runtime(wasm)
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

## 9. Milestone 5 — MCP and script runner lanes

After filesystem, extension discovery, WASM, and kernel composition work, add the other two V1 lanes.

### `crates/ironclaw_mcp`

Proves:

- MCP server manifest/discovery path
- stdio MCP tool discovery
- MCP tool to IronClaw capability mapping
- scoped invocation and audit hooks

### `crates/ironclaw_scripts`

Proves:

- `script.run` capability
- project-local Python/bash/JS helper execution
- sandbox profile limits
- scoped filesystem mounts
- no network/secrets by default

## 10. Milestone 6 — first-party product/userland extensions

Only after the runtime lanes work.

### `extensions/conversation`

Proves:

- normalized inbound schema
- channel-to-thread routing
- outbox paths
- configured agent-loop selection

### `extensions/missions`

Proves:

- filesystem-backed mission definitions
- cron/event/manual triggers
- dispatch/spawn into agent loops, scripts, or capabilities

### `extensions/agent_loop_tools`

Proves:

- agent loop as extension
- thread state in filesystem
- step append
- capability dispatch

### `extensions/gateway` and `extensions/tui`

Proves:

- channels as extensions
- reconnect/cursor/outbox model
- UI outside kernel

---

## 11. Milestone 7 — auth/network/sandbox hardening

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

## 12. Minimum viable vertical slice

The first meaningful proof should include:

```text
crates/ironclaw_filesystem
crates/ironclaw_extensions
crates/ironclaw_wasm
crates/ironclaw_kernel
wasm echo capability
```

End-to-end flow:

```text
filesystem mount
  -> discover echo WASM extension
  -> extract echo.say capability
  -> register capability with kernel host
  -> dispatch echo.say
  -> invoke WASM module
  -> emit runtime event
  -> return result
  -> write durable event/history if configured
```

This proves the architecture without product complexity.

---

## 13. Success criteria

The architecture is real when:

- `ironclaw_kernel` has almost no product logic
- `ironclaw_wasm` does not discover extensions
- `ironclaw_mcp` tools are adapted into capabilities and still go through policy/audit
- `ironclaw_scripts` is project-scoped and not a generic extension install path
- `ironclaw_extensions` does not execute capabilities
- `ironclaw_filesystem` does not know about agents
- `agent_loop` can be deleted or replaced without touching kernel
- `gateway` can be deleted or replaced without touching kernel
- WASM echo capability runs through the same path future installed WASM capabilities will use

---

## 14. Early architecture guardrails

Add guardrails as soon as the first crates exist:

- dependency checks between crates
- forbidden imports from extensions into kernel internals
- contract tests for manifests
- WASM host ABI tests
- MCP adapter tests
- script runner sandbox tests
- filesystem path traversal tests
- no outbound network bypasses in hosted mode
- no raw secret material in config fixtures

These tests are not polish. They are the mechanism that keeps the architecture from drifting.

---

## 15. Final recommendation

The next implementation work should be a sequence of small self-contained crates, not a broad product rewrite.

Start with the durable filesystem, then extension discovery, then WASM capability execution, then kernel composition, then a tiny WASM echo capability.

After that path is working, add MCP and script runner as the remaining V1 lanes. Only then should the team move conversation, missions, agent loop, gateway, TUI, auth, network, sandboxing, GitHub, or self-repair into the new model.
