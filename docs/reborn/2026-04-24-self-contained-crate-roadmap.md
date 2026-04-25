# IronClaw Reborn — Self-Contained Crate Roadmap

**Status:** Draft for review — local only until merged  
**Date:** 2026-04-24  
**Related docs:**

- `docs/reborn/2026-04-24-os-like-architecture-design.md`
- `docs/reborn/2026-04-24-architecture-faq-decisions.md`
- `docs/reborn/2026-04-24-existing-code-reuse-map.md`
- `docs/reborn/2026-04-24-host-api-invariants-and-authorization.md`

---

## 1. Purpose

Define the next implementation steps to make the OS-like IronClaw Reborn architecture real in a self-contained, crate-by-crate way.

The goal is not to rewrite IronClaw in one pass. The goal is to build a small vertical slice that proves the architecture:

```text
filesystem mount
  -> resolve tenant/user/project scope
  -> discover extension manifest
  -> register capability
  -> reserve resource budget
  -> execute a capability through WASM or the script runner
  -> reconcile resource budget
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

### PR 2 — `crates/ironclaw_host_api`

Define shared authority-bearing contracts from the host API invariants document: IDs, scopes, execution context, actions, decisions, paths, grants, approvals, resources, and audit envelopes.

### PR 3 — `crates/ironclaw_filesystem`

Build the durable path/mount API with explicit `/engine`, `/projects`, `/users`, `/memory`, and `/system/extensions` roots.

### PR 4 — `crates/ironclaw_resources`

Build the host-level resource/budget governor: tenant/user/project/mission/thread/invocation scopes, reserve/reconcile/release, and audit events.

### PR 5 — `crates/ironclaw_extensions`

Build manifest/discovery/capability declaration logic in its own crate.

### PR 6 — `crates/ironclaw_wasm` + budgeted WASM echo

Build the portable installed capability lane and prove one tiny WASM capability behind resource reservation.

### PR 7 — `crates/ironclaw_scripts` + Docker-backed script echo

Add `script.run` / declared CLI capability execution for native CLIs and project-local Python/bash/JS helpers. V1 uses Docker/container as the first backend.

### PR 8 — `crates/ironclaw_dispatcher`

Wire host API + filesystem + resources + extensions + WASM runtime + script runner into a composition-only host.

### PR 9 — `crates/ironclaw_mcp`

Adapt existing MCP servers/tools into IronClaw capabilities.

### PR 10 — `extensions/conversation` and `extensions/missions`

Add normalized inbound routing, channel-to-thread mapping, inbox/outbox semantics, and mission definition execution.

### PR 11 — `extensions/agent_loop_tools`

Move the default tool/capability agent loop into a first-party extension.

### PR 12 — `extensions/gateway` and `extensions/tui`

Move gateway/TUI channel behavior into first-party extensions and prove reconnect/cursor/outbox flow.

### PR 13 — auth/network/sandbox hardening

Add secret handles, mediated network, sandbox profile enforcement, and stronger scope propagation.

---

## 4. Milestone 0 — Freeze contracts in docs

Before coding each crate, write a short contract doc. Start from `docs/reborn/2026-04-24-host-api-invariants-and-authorization.md`; those invariants are the host API constitution, not optional implementation notes.

Suggested files:

```text
docs/reborn/contracts/host-api.md
docs/reborn/contracts/filesystem.md
docs/reborn/contracts/resources.md
docs/reborn/contracts/extensions.md
docs/reborn/contracts/wasm.md
docs/reborn/contracts/dispatcher.md
docs/reborn/contracts/capability-access.md
docs/reborn/contracts/capabilities.md
docs/reborn/contracts/run-state.md
docs/reborn/contracts/approvals.md
docs/reborn/contracts/live-vertical-slice.md
docs/reborn/contracts/mcp.md
docs/reborn/contracts/scripts.md
docs/reborn/contracts/processes.md
docs/reborn/contracts/auth.md
docs/reborn/contracts/network.md
docs/reborn/contracts/events.md
docs/reborn/contracts/live-vertical-slice.md
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

## 5. Milestone 1 — `ironclaw_host_api`

### Purpose

Define shared authority-bearing contracts before any service or runtime crate can drift.

### Crate

```text
crates/ironclaw_host_api/
```

### Build

- identity/scope newtypes: tenant, user, project, mission, thread, invocation, process, extension
- capability IDs, descriptors, grants, and grant constraints
- `ExecutionContext`
- path contracts: host path, virtual path, scoped path, mount alias, mount view
- runtime/trust enums
- `Action`, `Decision`, `DenyReason`, `ApprovalRequest`, and `Obligation`
- resource scope, estimate, and usage contracts
- audit envelope and correlation IDs

### Tests

- invalid IDs/names are rejected
- scoped path strings cannot represent raw host paths
- action/decision types serialize with stable names
- child grants cannot be constructed with obviously broader authority than parents in helper constructors

### Non-goals

Do not add:

- filesystem implementation
- resource ledger enforcement
- extension discovery
- runtime execution
- product workflows

## 6. Milestone 2 — `ironclaw_filesystem`

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
mount("/engine", db_or_local_backend)
mount("/system/extensions", local_backend)
mount("/users", db_or_local_backend)
mount("/projects", local_backend)
mount("/memory", db_or_remote_backend)
```

### Tests

- cannot escape mounted root with `..`
- read/write roundtrip
- list/stat work
- mount routing works
- path normalization is deterministic
- missing mount returns a typed error
- default namespace exposes `/engine`, `/projects`, `/users`, `/memory`, and `/system/extensions` roots

### Non-goals

Do not add:

- search/indexing
- transactions beyond backend-local atomic writes
- subscriptions/watch
- auth policy
- secret storage
- thread orchestration

---

## 7. Milestone 3 — `ironclaw_resources`

### Purpose

Enforce multi-tenant resource budgets and quotas before runtime lanes can spend money or consume scarce host resources.

### Crate

```text
crates/ironclaw_resources/
```

### Build

- scope cascade: tenant/org, user, project, mission, thread, sub-thread/invocation
- `reserve`, `reconcile`, and `release` API
- budget/resource ledger records
- budget warning/approval/denial events
- hard invariant caps
- V1 resource model: USD, tokens, wall-clock, concurrency, output bytes, process count
- sandbox quota contract for CPU, memory, disk, and network

### API sketch

```rust
async fn reserve(
    scopes: &[ResourceScope],
    estimate: ResourceEstimate,
) -> Result<ResourceReservation, ResourceDenial>;

async fn reconcile(
    reservation: ResourceReservation,
    actual: ResourceUsage,
) -> Result<()>;

async fn release(reservation: ResourceReservation) -> Result<()>;
```

### Tests

- reservation denied when tenant/user/project is exhausted
- reservation succeeds only if every scope has capacity
- reconciliation releases over-reservation
- release does not record spend
- concurrent reservations cannot oversubscribe one scope
- zero-dollar/local model still respects token and runtime quota limits

### Non-goals

Do not add:

- billing/payment integration
- LLM provider implementation
- product UI
- progress/stuck-loop heuristics as budget substitutes

---

## 8. Milestone 4 — `ironclaw_extensions`

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

## 9. Milestone 5 — `ironclaw_wasm`

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
    ctx: ExecutionContext,
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

## 10. Milestone 6 — `ironclaw_scripts`

### Purpose

Provide the native CLI/software lane without requiring the world's CLIs to be rebuilt in WASM.

### Crate

```text
crates/ironclaw_scripts/
```

### Build

- `script.run` and declared CLI capability contracts
- Docker/container-backed V1 sandbox backend
- command/argument/environment allowlist translation
- scoped filesystem mount preparation
- network deny-by-default with explicit allow rules
- secret-handle injection through approved env/files only
- CPU/memory/PID/wall-clock/output limits
- artifact directory capture and cleanup

### Tests

- approved command runs in the configured image/backend
- raw Docker flags cannot be requested by an extension
- scoped filesystem mounts are passed read-only/read-write as authorized
- network and secrets are denied by default
- stdout/stderr/output size limits are enforced
- artifacts are exported only from approved paths

### Non-goals

Do not add:

- arbitrary host shell access
- multiple sandbox backends in V1
- Docker socket exposure to extensions
- MCP protocol handling
- product workflows

---

## 11. Milestone 7 — `ironclaw_dispatcher`

### Purpose

Compose the system.

### Crate

```text
crates/ironclaw_dispatcher/
```

### Build

- system builder
- host API + filesystem + resources + extension manager + WASM runtime + script runner wiring
- event bus composition
- boot namespace
- extension capability registration into host dispatch table

### API sketch

```rust
let kernel = KernelBuilder::new()
    .with_host_api_contracts(host_api)
    .with_filesystem(fs)
    .with_resource_governor(resources)
    .with_extension_manager(extensions)
    .with_wasm_runtime(wasm)
    .with_script_runner(scripts)
    .build()
    .await?;
```

### Tests

- boot creates namespace
- discovers extension
- registers capabilities
- dispatches discovered capability
- dispatches a script/CLI capability through the same host path
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

## 12. Milestone 8 — `ironclaw_mcp`

Proves:

- MCP server manifest/discovery path
- stdio MCP tool discovery
- MCP tool to IronClaw capability mapping
- scoped invocation and audit hooks

MCP is an adapter path for existing ecosystems; local stdio servers should reuse the same mediated process/sandbox substrate as script runner where appropriate.

---

## 13. Milestone 9 — first-party product/userland extensions

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

## 14. Milestone 10 — auth/network/sandbox hardening

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

## 15. Minimum viable vertical slice

The first meaningful proof should include:

```text
crates/ironclaw_host_api
crates/ironclaw_filesystem
crates/ironclaw_resources
crates/ironclaw_extensions
crates/ironclaw_wasm
crates/ironclaw_scripts
crates/ironclaw_dispatcher
crates/ironclaw_authorization
crates/ironclaw_capabilities
crates/ironclaw_run_state
crates/ironclaw_approvals
crates/ironclaw_processes
wasm echo capability
script echo capability
```

End-to-end flow:

```text
filesystem mount
  -> discover echo WASM extension
  -> extract echo.say capability
  -> register capability with kernel host
  -> dispatch echo.say
  -> reserve tenant/user/project/thread budget
  -> invoke WASM module or Docker-backed script command
  -> reconcile actual resource usage
  -> emit runtime event
  -> return result
  -> write durable event/history if configured
```

This proves the architecture without product complexity.

---

## 16. Success criteria

The architecture is real when:

- `ironclaw_host_api` has no runtime/product logic
- `ironclaw_dispatcher` has no authorization/product workflow logic; it only routes already-authorized dispatches
- `ironclaw_capabilities` is the caller-facing invocation workflow between authorization, run-state, process start, and dispatch
- `ironclaw_approvals` resolves pending approval records into scoped leases without touching dispatcher/runtime execution
- `ironclaw_authorization` enforces scoped lease visibility, expiration, claim, consumption, and revocation before dispatch
- fingerprinted approval leases are resume-only and cannot authorize plain `invoke_json` as ambient grants
- `ironclaw_capabilities` binds approval-required dispatches to an invocation fingerprint before persistence/resume
- `CapabilityHost::resume_json` resumes approved dispatches through the same authorization/dispatcher path, claims the matching lease before dispatch, and consumes it after success
- run-state `start` rejects duplicate tenant/user/invocation records instead of overwriting lifecycle state
- `Action::SpawnCapability` is capability-targeted, not extension-level or raw-process authority
- `CapabilityHost::spawn_json` authorizes `SpawnProcess` plus target capability effects before creating a tracked process record
- `ironclaw_processes` stores tenant/user-scoped `ProcessRecord` lifecycle state without owning authorization policy
- `BackgroundProcessManager` can run spawned work through `ProcessExecutor` and update process state on executor success/failure without letting late completion overwrite killed processes
- `ProcessHost` exposes host-facing `status`, `kill`, `await_process`, `subscribe`, `result`, and `await_result` APIs over scoped process current state/results without moving process lifecycle back into `CapabilityHost`
- `ProcessCancellationRegistry` lets `ProcessHost::kill` signal scoped cooperative cancellation tokens for background executors without allowing cross-tenant cancellation
- `ProcessResultStore` records scoped terminal process output/error records; in-memory/dev stores can keep small JSON inline, while filesystem-backed stores write successful JSON output to scoped output refs for reviewable storage hygiene
- `EventingProcessStore` emits tenant/user-scoped process_started/process_completed/process_failed/process_killed events without making dispatcher process-aware
- `ResourceManagedProcessStore` reserves resources before process start, records reservation IDs, reconciles on completion, and releases on failure/kill/start failure
- `ironclaw_resources` is the only path for costed/quota-limited invocation accounting
- `ironclaw_wasm` does not discover extensions
- `ironclaw_mcp` tools are adapted into capabilities and still go through policy/audit
- `ironclaw_scripts` is project-scoped and not a generic extension install path
- `ironclaw_extensions` does not execute capabilities
- `ironclaw_filesystem` does not know about agents
- `agent_loop` can be deleted or replaced without touching kernel
- `gateway` can be deleted or replaced without touching kernel
- WASM echo capability runs through the same path future installed WASM capabilities will use
- script echo capability runs through the same policy/resource/audit path as existing native CLIs

---

## 17. Early architecture guardrails

Add guardrails as soon as the first crates exist:

- host API invariant tests
- dependency checks between crates
- forbidden imports from extensions into kernel internals
- contract tests for manifests
- resource reservation/concurrency tests
- WASM host ABI tests
- MCP adapter tests
- script runner sandbox tests
- filesystem path traversal tests
- no outbound network bypasses in hosted mode
- no raw secret material in config fixtures

These tests are not polish. They are the mechanism that keeps the architecture from drifting.

---

## 18. Final recommendation

The next implementation work should be a sequence of small self-contained crates, not a broad product rewrite.

Start with `ironclaw_host_api`, then the durable filesystem, then the resource/budget governor, then extension discovery, then WASM capability execution, then Docker-backed script runner execution, then kernel composition, then tiny budgeted WASM and script echo capabilities.

After those paths are working, add MCP as the required adapter lane for existing MCP servers/tools. Only then should the team move conversation, missions, agent loop, gateway, TUI, auth, network hardening, GitHub, or self-repair into the new model.
