# IronClaw Reborn — Existing Code Reuse Map

**Status:** Draft for implementation planning  
**Date:** 2026-04-24  
**Related docs:**

- `docs/reborn/2026-04-24-os-like-architecture-design.md`
- `docs/reborn/2026-04-24-architecture-faq-decisions.md`
- `docs/reborn/2026-04-24-self-contained-crate-roadmap.md`
- `docs/reborn/2026-04-24-host-api-invariants-and-authorization.md`

---

## 1. Purpose

Reborn should not reinvent everything. The current IronClaw codebase already has strong implementations for WASM, MCP, skills, safety, workspace mounts, OAuth, tool permissions, missions, gateway UI, and TUI components.

The goal of this map is to identify what to extract, what to adapt, and what to leave behind while implementing the Reborn architecture.

Core rule:

```text
Reuse proven internals behind new boundaries.
Do not preserve today's coupling just because the code works.
```

---

## 2. Reuse principles

1. **Extract behavior, not dependency shape.**  
   Existing modules can seed new crates, but the new crates should follow the Reborn dependency graph.

2. **Contracts first.**  
   Shared types move into `ironclaw_host_api` before runtime crates start depending on them.

3. **Tests move with the extracted behavior.**  
   Path traversal, WASM limits, MCP auth, skill validation, approval gates, and cost calculations already have regression value. Preserve or port those tests when extracting.

4. **Keep runtime lanes separate.**  
   WASM and MCP can be extension package types, but their execution/adaptation code should stay in `ironclaw_wasm` and `ironclaw_mcp`, not inside `ironclaw_extensions`.

5. **Prefer thin adapters during migration.**  
   The first Reborn crates can wrap existing code temporarily, but the public API should be the new Reborn API.

6. **Do not carry forward arbitrary iteration caps as product semantics.**  
   Iteration/time caps can remain hard safety invariants, but normal autonomous stopping should be budget/progress driven.

---

## 3. Reuse levels

| Level | Meaning | Use when |
|---|---|---|
| Direct seed | Existing code can mostly move into a new crate with imports/types adjusted | WASM runtime, MCP protocol/transports, skill parser/registry |
| Extract with surgery | Existing code is valuable but currently entangled with agent/tool/runtime systems | Extension manager, missions, auth/secrets, gateway surfaces |
| Reference only | Existing code demonstrates behavior but should not become the new implementation | Current `CostGuard`, max-iteration controls, current agent loop orchestration |
| Do not carry forward | Existing behavior conflicts with Reborn goals | Blob-style manager ownership, hidden runtime bypasses, kernel product logic |

---

## 4. Crate-by-crate reuse map

### 4.1 `crates/ironclaw_host_api`

**Purpose in Reborn:** shared contracts, IDs, scopes, descriptors, and envelope types. No behavior.

**Existing sources:**

- `crates/ironclaw_common/src/identity.rs`
- `crates/ironclaw_common/src/event.rs`
- `crates/ironclaw_common/src/timezone.rs`
- `crates/ironclaw_common/src/util.rs`
- `crates/ironclaw_engine/src/types/capability.rs`
- `crates/ironclaw_engine/src/types/project.rs`
- `crates/ironclaw_engine/src/types/thread.rs`
- `crates/ironclaw_engine/src/types/mission.rs`
- `crates/ironclaw_engine/src/types/event.rs`
- `crates/ironclaw_engine/src/types/step.rs`
- `crates/ironclaw_engine/src/types/conversation.rs`
- `crates/ironclaw_engine/src/types/provenance.rs`

**Keep:**

- validated identity/name newtype patterns
- UUID newtype patterns for IDs
- existing serialization conventions where compatible
- capability/effect concepts
- event envelope ideas
- timezone validation helpers

**Refactor:**

- split pure contract types from engine runtime state
- add `TenantId` / `OrgId` from day one
- add `ExecutionContext`, `ResourceScope`, `MountView`, `RuntimeSpec`, and `CapabilityDescriptor`
- move approval/budget-denial wire enums here only if they are host-facing contracts

**Do not carry forward:**

- `ThreadConfig::max_iterations` as a normal product stop condition
- `ThreadConfig::max_budget_usd` as the final budget model
- `MAX_WORKER_ITERATIONS` from `crates/ironclaw_common/src/lib.rs`

---

### 4.2 `crates/ironclaw_filesystem`

**Purpose in Reborn:** durable path/mount API and scoped namespace.

**Existing sources:**

- `crates/ironclaw_engine/src/workspace/mount.rs`
- `crates/ironclaw_engine/src/workspace/filesystem.rs`
- `crates/ironclaw_engine/src/workspace/registry.rs`
- `src/workspace/document.rs`
- `src/workspace/extension_state.rs`
- `src/workspace/layer.rs`
- `src/workspace/repository.rs`
- `src/workspace/settings_adapter.rs`
- `src/workspace/settings_schemas.rs`

**Keep:**

- `MountBackend` trait shape as a seed for backend abstraction
- typed mount errors
- `read` / `write` / `list` / `stat` style API
- lexical path validation
- rejection of absolute paths and `..`
- symlink escape defense from `FilesystemBackend`
- extension state path validation patterns

**Refactor:**

- replace current workspace-relative `.system/...` assumptions with Reborn roots:

```text
/engine
/system/extensions
/users
/projects
/memory
```

- add scoped aliases:

```text
/project
/workspace
/memory
/extension/config
/extension/state
/extension/cache
/tmp
```

- separate `RootFilesystem` from `ScopedFilesystem`
- ensure extensions see only scoped aliases and explicit `MountView`s
- make project and memory roots first-class rather than helper modules around the old workspace abstraction

**Do not carry forward:**

- shell/patch passthrough behavior as part of the core filesystem contract
- search/indexing as filesystem responsibilities
- raw host paths in extension-visible APIs

---

### 4.3 `crates/ironclaw_resources`

**Purpose in Reborn:** multi-tenant resource and budget governor.

**Existing sources:**

- `src/agent/cost_guard.rs`
- `src/llm/costs.rs`
- `crates/ironclaw_engine/src/runtime/mission.rs` (`BudgetGate` concept)
- `crates/ironclaw_engine/src/types/thread.rs` cost/token counters
- `crates/ironclaw_engine/src/types/step.rs` usage/cost fields
- existing DB usage/cost fields in `src/db/*`

**Keep:**

- model pricing table and token cost calculation from `src/llm/costs.rs`
- per-user/day spend concept
- warning threshold concept
- mission-level budget gate concept
- token and cost telemetry fields

**Refactor:**

- replace in-memory counters with a ledger/reservation model
- include tenant/org in the scope cascade:

```text
tenant/org -> user -> project -> mission -> thread -> sub-thread/invocation
```

- implement:

```text
reserve(scope, estimate) -> execute -> reconcile(actual) / release()
```

- enforce atomic reservations in the database for hosted/multi-tenant mode
- support USD as primary ledgered budget and tokens/wall-clock/concurrency/output/process-count as V1 resource dimensions
- model CPU/memory/disk/network initially as sandbox quota descriptors

**Do not carry forward as final design:**

- current `CostGuard` as the enforcement engine; it is useful reference code but in-memory and not reservation-based
- `max_iterations` as a budget substitute
- silent timeout death instead of explicit budget/progress events

---

### 4.4 `crates/ironclaw_extensions`

**Purpose in Reborn:** discover extension packages, validate manifests/layouts, and declare capabilities. It knows what can run, not how to run it.

**Existing sources:**

- `src/extensions/mod.rs`
- `src/extensions/naming.rs`
- `src/extensions/registry.rs`
- `src/extensions/discovery.rs`
- `src/workspace/extension_state.rs`
- `src/tools/builtin/extension_tools.rs` for user-facing install/search/auth UX patterns

**Keep:**

- extension name canonicalization and validation
- curated registry and fuzzy search patterns
- online MCP discovery ideas
- extension kind/source/auth-hint concepts
- durable config/state schema ideas
- install/search/list/remove UX lessons

**Refactor:**

- move manifest/runtime declarations into `ironclaw_host_api` or `ironclaw_extensions` contract types
- make runtime lane explicit: `wasm`, `mcp`, `script`, or first-party/system extension
- let `ironclaw_extensions` produce `CapabilityDescriptor`s, not runtime objects
- keep extension-local folders explicit:

```text
/system/extensions/<extension>/manifest.toml
/system/extensions/<extension>/capabilities.json
/system/extensions/<extension>/skills/
/system/extensions/<extension>/scripts/
/system/extensions/<extension>/wasm/
/system/extensions/<extension>/config/
/system/extensions/<extension>/state/
/system/extensions/<extension>/cache/
```

**Do not carry forward wholesale:**

- `src/extensions/manager.rs` as the new extension manager. It currently mixes extension registry, channels, WASM, MCP, secrets, tools, hooks, pairing, OAuth, installation, activation, and runtime hot-activation.

**Target boundary:**

```text
ironclaw_extensions = package discovery + manifest/capability declarations
ironclaw_wasm       = WASM execution
ironclaw_mcp        = MCP adaptation/execution
ironclaw_auth       = credentials/OAuth
ironclaw_kernel     = composition
```

---

### 4.5 `crates/ironclaw_wasm`

**Purpose in Reborn:** default installed capability runtime for stable reusable capabilities.

**Existing sources:**

- `src/tools/wasm/mod.rs`
- `src/tools/wasm/runtime.rs`
- `src/tools/wasm/loader.rs`
- `src/tools/wasm/limits.rs`
- `src/tools/wasm/host.rs`
- `src/tools/wasm/capabilities.rs`
- `src/tools/wasm/capabilities_schema.rs`
- `src/tools/wasm/http_security.rs`
- `src/tools/wasm/credential_injector.rs`
- `src/tools/wasm/storage.rs`
- `src/tools/wasm/rate_limiter.rs`
- `src/tools/wasm/wrapper.rs`

**Keep:**

- Wasmtime engine setup
- component model support
- compile-once / instantiate-fresh pattern
- fuel metering
- epoch interruption timeout backup
- memory/resource limits
- compilation cache handling
- BLAKE3 binary integrity checks
- capability schema parsing
- SSRF-safe HTTP helpers
- credential injection at host boundary
- per-tool rate limit ideas
- existing security tests/fuzz coverage where applicable

**Refactor:**

- replace old `Tool` wrapper output with Reborn `CapabilityInvocation`
- make resource reservation mandatory at invocation boundaries
- replace workspace host imports with scoped filesystem imports
- route network/auth through `ironclaw_network` and `ironclaw_auth`
- align capabilities schema with Reborn manifest/capability descriptors

**Do not carry forward:**

- direct registration into current `ToolRegistry` as the primary integration point
- WASM channel/product assumptions in the base WASM capability runtime unless explicitly modeled as a channel extension later

---

### 4.6 `crates/ironclaw_mcp`

**Purpose in Reborn:** adapt MCP servers/tools into IronClaw capabilities.

**Existing sources:**

- `src/tools/mcp/mod.rs`
- `src/tools/mcp/protocol.rs`
- `src/tools/mcp/client.rs`
- `src/tools/mcp/config.rs`
- `src/tools/mcp/factory.rs`
- `src/tools/mcp/session.rs`
- `src/tools/mcp/transport.rs`
- `src/tools/mcp/http_transport.rs`
- `src/tools/mcp/stdio_transport.rs`
- `src/tools/mcp/unix_transport.rs`
- `src/tools/mcp/process.rs`
- `src/tools/mcp/auth.rs`
- `src/tools/mcp/client_store.rs`

**Keep:**

- MCP protocol types
- tool discovery/listing logic
- HTTP transport
- stdio transport
- Unix transport where supported
- session manager
- OAuth/auth flow helpers
- process manager for stdio MCP
- factory/config parsing lessons
- auth-error detection tests

**Refactor:**

- represent MCP tools as Reborn `CapabilityDescriptor`s
- route calls through `ironclaw_resources` before execution
- route secrets through `ironclaw_auth`
- route remote HTTP through `ironclaw_network`
- keep stdio process handling as an internal process substrate, not a public arbitrary process extension lane

**Do not carry forward:**

- direct `ToolRegistry` registration as the long-term boundary
- MCP-specific auth or network bypasses outside system-service mediation

---

### 4.7 `crates/ironclaw_scripts`

**Purpose in Reborn:** dynamic creativity lane for project-local Python/bash/JS helpers.

**Existing sources:**

- `src/tools/builtin/shell.rs`
- `src/worker/job.rs`
- `src/worker/container.rs`
- `crates/ironclaw_engine/src/executor/scripting.rs`
- `crates/ironclaw_engine/src/executor/orchestrator.rs` for lessons about Python orchestration limits
- `crates/ironclaw_engine/src/runtime/internal_write.rs` for self-modification safety lessons

**Keep:**

- shell execution safety lessons
- output limits and truncation patterns once implemented
- container/job lifecycle concepts
- project sandbox lifecycle lessons
- `llm_query` recursion budget inheritance concept, but under `ironclaw_resources`

**Refactor:**

- expose one V1 capability such as `script.run`
- require explicit inputs, mounts, network policy, timeout, memory/output limit, and artifact directory
- enforce resource reservation and sandbox quotas
- make scripts project-scoped by default
- treat generated scripts as artifacts that can later be promoted to WASM/MCP/stable capabilities

**Do not carry forward as foundation:**

- Monty as the primary scripting answer
- CodeAct as the default agent loop
- arbitrary generic process extensions as a public V1 substrate

---

### 4.8 `crates/ironclaw_auth`

**Purpose in Reborn:** identity, credentials, OAuth, secret handles, and short-lived secret injection.

**Existing sources:**

- `src/auth/*`
- `src/secrets/*`
- `src/tools/builtin/secrets_tools.rs`
- `src/tools/wasm/credential_injector.rs`
- `src/tools/mcp/auth.rs`
- `src/tools/mcp/session.rs`
- `src/pairing/*`

**Keep:**

- secret handle patterns
- OAuth flow descriptors
- pending OAuth launch/approval flow ideas
- MCP OAuth helpers
- credential injection at host boundary
- redaction behavior
- pairing/approval UX lessons

**Refactor:**

- raw secrets never live in filesystem config
- extension config references secret handles only
- all runtime lanes request short-lived secret material through `ironclaw_auth`
- audit secret resolution/injection boundaries

**Do not carry forward:**

- runtime crates reading raw secret storage directly
- extension-local config containing raw credential values

---

### 4.9 `crates/ironclaw_network`

**Purpose in Reborn:** mediated outbound network boundary.

**Existing sources:**

- `src/tools/builtin/http.rs`
- `src/tools/wasm/http_security.rs`
- `src/tools/mcp/http_transport.rs`
- `src/tunnel/*` where useful for deployment/network lessons
- `crates/ironclaw_safety/src/credential_detect.rs`
- `crates/ironclaw_safety/src/leak_detector.rs`

**Keep:**

- SSRF/private-IP rejection
- allowlist validation patterns
- request timeout defaults
- output/sanitization lessons
- credential/leak scanning around network outputs

**Refactor:**

- expose a mediated host network API instead of arbitrary HTTP helpers
- make outbound policy scope-aware: tenant/user/project/extension/capability
- integrate with `ironclaw_resources` for network quota/egress accounting when implemented

**Do not carry forward:**

- direct HTTP clients from runtime lanes that bypass policy/audit

---

### 4.10 Skills as filesystem-backed cognitive artifacts

**Purpose in Reborn:** skills guide workflows; they do not replace stable capabilities.

**Existing sources:**

- `crates/ironclaw_skills/src/types.rs`
- `crates/ironclaw_skills/src/parser.rs`
- `crates/ironclaw_skills/src/registry.rs`
- `crates/ironclaw_skills/src/selector.rs`
- `crates/ironclaw_skills/src/gating.rs`
- `crates/ironclaw_skills/src/validation.rs`
- `crates/ironclaw_skills/src/v2.rs`

**Keep:**

- `SKILL.md` parser
- activation criteria
- trust levels
- gating requirements
- prompt budget checks
- name/path validation
- selector logic
- installed vs trusted source distinction

**Refactor:**

- load from Reborn filesystem roots:

```text
/system/skills
/users/<user>/skills
/projects/<project>/skills
/system/extensions/<extension>/skills
```

- make skill selection use `ExecutionContext` and `MountView`

**Do not carry forward:**

- any assumption that skills directly execute privileged actions

---

### 4.11 Missions as first-party extension

**Purpose in Reborn:** mission engine runs durable mission definitions through host dispatch/spawn.

**Existing sources:**

- `crates/ironclaw_engine/src/types/mission.rs`
- `crates/ironclaw_engine/src/runtime/mission.rs`
- `src/agent/routine_engine.rs` for routine-to-mission migration lessons
- `crates/ironclaw_gateway/static/styles/surfaces/missions.css`

**Keep:**

- mission lifecycle states
- cron/event/system-event/webhook/manual cadence concepts
- cooldown
- dedup window
- max concurrent
- per-user fire-rate limit
- notification model
- mission budget gate concept

**Refactor:**

- mission definitions become filesystem records:

```text
/projects/<project>/missions/*.toml
/users/<user>/missions/*.toml
/system/missions/*.toml
```

- mission firing uses host dispatch/spawn, not direct `ThreadManager` coupling
- per-invocation budget comes from `ironclaw_resources`
- mission outputs are durable artifacts/events under filesystem paths

**Do not carry forward:**

- tight coupling to `ThreadManager`, `Store`, `RetrievalEngine`, or `SkillTracker` as the first-party extension boundary

---

### 4.12 Gateway, TUI, and approval UX

**Purpose in Reborn:** first-party channel/UI extensions after the core vertical slice works.

**Existing sources:**

- `crates/ironclaw_gateway/*`
- `crates/ironclaw_gateway/static/js/core/*`
- `crates/ironclaw_gateway/static/js/surfaces/*`
- `crates/ironclaw_tui/*`
- `src/gate/approval.rs`
- `crates/ironclaw_engine/src/gate/*`
- `src/tools/permissions.rs`

**Keep:**

- existing web surfaces as UI reference/components
- SSE/reconnect/outbox lessons
- TUI widgets for approvals, conversations, tools, thread lists
- approval gate concepts
- per-tool permission model
- admin-disabled tool policy

**Refactor:**

- gateway and TUI become transport/UI extensions over filesystem-backed thread/outbox state
- budget approval gates share the same gate/resolve pattern as tool approvals
- durable truth lives in filesystem/audit/events, not UI memory

**Do not carry forward:**

- gateway-specific state as the source of truth
- kernel dependencies on UI surfaces

---

### 4.13 Safety and policy

**Purpose in Reborn:** cross-cutting safety primitives used by host services and runtime lanes.

**Existing sources:**

- `crates/ironclaw_safety/*`
- `src/tools/permissions.rs`
- `src/gate/approval.rs`
- `crates/ironclaw_engine/src/gate/*`
- `src/tools/redaction.rs`
- `src/tools/schema_validator.rs`
- `src/tools/coercion.rs`

**Keep:**

- credential detection
- leak detection
- sanitizer/validator
- sensitive path logic
- prompt/tool parameter validation
- schema validation
- parameter coercion lessons
- approval gate pipeline
- admin/user permission policy

**Refactor:**

- make safety calls explicit host boundary steps
- policy decisions include `ExecutionContext`, `CapabilityDescriptor`, and declared effects
- approval and resource gates should produce auditable events

**Do not carry forward:**

- hidden bypass paths for internal callers
- policy checks that only happen in one agent loop path

---

## 5. Things to avoid carrying forward

The following current patterns should not become foundations of Reborn:

1. `max_iterations` as the normal autonomous stop mechanism.
2. `MAX_WORKER_ITERATIONS = 500` as a product-level budget.
3. In-memory `CostGuard` as the final multi-tenant enforcement layer.
4. Current `ExtensionManager` as a combined registry/runtime/auth/channel manager.
5. Direct `ToolRegistry` as the central abstraction for every capability type.
6. Agent loop as core runtime/kernel behavior.
7. CodeAct/Monty as foundational execution lanes.
8. Gateway/TUI assumptions inside core crates.
9. Runtime lanes reading raw secrets or opening raw network clients directly.
10. Extension-visible raw host paths.
11. Product workflows accumulating in `ironclaw_kernel`.

---

## 6. Recommended extraction order

Start with the pieces that create safe boundaries before extracting product behavior.

```text
1. ironclaw_host_api
   from ironclaw_common + selected ironclaw_engine/src/types

2. ironclaw_filesystem
   from ironclaw_engine/src/workspace/{mount,filesystem,registry}.rs
   plus selected src/workspace path/schema lessons

3. ironclaw_resources
   from cost_guard/cost tables/usage telemetry as reference
   but with new reservation ledger design

4. ironclaw_extensions
   from src/extensions/{naming,registry,discovery}.rs
   plus workspace extension_state path/schema lessons

5. ironclaw_wasm
   from src/tools/wasm/*

6. ironclaw_mcp
   from src/tools/mcp/*

7. ironclaw_scripts
   from shell/job/container/scripting lessons

8. first-party extensions
   conversation, missions, agent_loop_tools, gateway, TUI
```

The first proof should still be the budgeted WASM echo vertical slice:

```text
mount /system/extensions/echo
  -> discover manifest
  -> register echo.say capability
  -> reserve tenant/user/project/thread budget
  -> invoke WASM
  -> reconcile resource usage
  -> emit runtime/audit events
  -> return result
```

---

## 7. Ported test checklist

When extracting, preserve or recreate tests for:

- path traversal rejection
- symlink escape prevention
- mount routing
- extension name canonicalization
- invalid extension path rejection
- skill manifest validation
- skill prompt budget enforcement
- WASM fuel/time/memory/output limits
- WASM binary integrity verification
- WASM HTTP allowlist / SSRF rejection
- MCP auth error recognition
- MCP transport selection
- approval gate behavior by execution mode
- admin-disabled tool policy validation
- cost calculation from token usage
- resource reservation concurrency once `ironclaw_resources` exists
- mission cooldown/dedup/concurrency behavior when missions are extracted

---

## 8. Implementation warning

The current codebase has many useful parts, but several of them are useful because they encode hard-earned edge cases, not because their current module ownership is correct.

When in doubt:

```text
copy the invariant
copy the test
change the boundary
```
