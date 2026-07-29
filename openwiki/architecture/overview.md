# Architecture Overview

This page explains IronClaw's system design, the four-layer model, dependency structure, and where to build new features.

## High-Level System Design

IronClaw is a **secure personal AI assistant** that executes agent workflows in sandboxed environments with policy-enforced access to tools and external services. The system prioritizes:

1. **Security first:** Encrypted secrets, sandboxed execution, approval gates
2. **Modularity:** Clear component boundaries, composable layers, trait-based extensibility
3. **Durability:** Event sourcing, snapshot recovery, multi-backend support (PostgreSQL, libSQL)
4. **Observability:** Structured logging, event tracing, audit trails

## The Dual Stack: v1 and Reborn

IronClaw runs two architectures in parallel:

### v1 (Legacy, Maintenance Only)
- **Location:** `src/` directory (~10k LOC)
- **Model:** Monolith with tightly coupled modules
- **Status:** Deprecated; being phased out
- **When to touch:** Only to maintain existing v1 behavior
- **New features:** ❌ Do not add features to v1

### Reborn (Modern, Active Development)
- **Location:** `crates/` directory (68+ focused crates)
- **Model:** Modular architecture with clear authority boundaries
- **Status:** Primary target for new development
- **When to touch:** All new features go here
- **Migration:** Reborn replaces v1 gradually without forking the user experience

**Key Rule:** Build new features in Reborn (`crates/`), not v1 (`src/`).

## The Four-Layer Model (Reborn)

Reborn uses a kernel-userland architecture inspired by operating systems:

```
┌─────────────────────────────────────────────────────────────┐
│                        Products Layer                        │
│  CLI, WebUI, Slack, Telegram, custom channels & adapters    │
│                  (UX and surface ownership)                  │
├──────────────── TurnCoordinator Boundary ──────────────────┤
│                  Userland: Agent Loops                       │
│  Planned Agentic, Text, CodeAct                             │
│  (request effects through host ports)                       │
├──────────────── CapabilityHost Boundary ──────────────────┤
│             Kernel: Authority & Policy Gates                │
│  Authorization (who can access what)                        │
│  Approvals (human sign-off for dangerous ops)              │
│  Safety (prompt injection, credential detection)           │
│  Secrets (encrypted storage, injected at transit)          │
│  Resources (bounded execution, cost tracking)              │
│  Filesystem (file scoping, integrity)                      │
├──────────────── Effect Subscription Boundary ───────────────┤
│              Substrates: Durable Primitives                 │
│  Events (immutable audit log)                              │
│  Threads (turn history and state)                          │
│  Filesystem (user data, attachments)                       │
│  Memory (embeddings, search index)                         │
│  Run State (checkpoints, recovery)                         │
└─────────────────────────────────────────────────────────────┘
```

### Layer Responsibilities

| Layer | Owns | Does NOT Own | Boundary |
|-------|------|--------------|----------|
| **Products** | CLI/WebUI/channel UX | Agent logic, tools, DB | HTTP, channel-to-turn translation |
| **Userland (Loops)** | Planning, reasoning, tool selection | Security, approvals, secrets | Host ports (request only) |
| **Kernel (Gates)** | Policy, security, approvals, secrets | Loop implementation | Effects isolation |
| **Substrates** | Persistence, durability, indexing | Policy, security, approval logic | Backend-agnostic traits |

### Core Principle

**The loop is NOT the security perimeter.** Loops request effects through host ports; the kernel **decides what's allowed**. This means:

- Loops cannot directly call databases, filesystems, or secrets
- Every capability request passes through authorization gates
- Approvals are scoped to exact invocations, not blanket grants
- The kernel can deny, modify, or delay any requested effect

## Crate Organization

IronClaw's 68+ crates are organized into 7 functional groups:

### 1. Core Contracts (5 crates)
**Purpose:** Shared types, traits, and interfaces used across the system.

| Crate | Purpose |
|-------|---------|
| `ironclaw_host_api` | Traits and types for loop-to-kernel communication (HostPort, CapabilityRequest, etc.) |
| `ironclaw_common` | Shared utilities, types, and enums (Attachment, Event, Identity, etc.) |
| `ironclaw_prompt_envelope` | Prompt composition, template system, and injection safety |
| `ironclaw_runtime_policy` | Policy types, profile definitions, and validation |
| `ironclaw_architecture` | Architecture boundary tests and dependency checks |

### 2. Authority & Gates (9 crates)
**Purpose:** Policy enforcement, security, and approval gates.

| Crate | Purpose |
|-------|---------|
| `ironclaw_authorization` | Who can access what (RBAC, permission checks) |
| `ironclaw_approvals` | Human approval flows and lease management |
| `ironclaw_trust` | Trust boundaries and identity verification |
| `ironclaw_resources` | Resource governor, cost tracking, quotas |
| `ironclaw_secrets` | Encrypted secret storage and injection |
| `ironclaw_safety` | Prompt injection, credential detection, sanitization |
| `ironclaw_network` | Network sandbox, allowlist/denylist, DNS |
| `ironclaw_filesystem` | File scoping, integrity checks, namespace isolation |
| `ironclaw_hooks` | Lifecycle hooks, event subscribers, plugins |

### 3. Capability Execution (11 crates)
**Purpose:** Tool registration, dispatch, and execution in sandboxes.

| Crate | Purpose |
|-------|---------|
| `ironclaw_capabilities` | Capability registry, profile conformance, host API |
| `ironclaw_dispatcher` | Multi-destination dispatch (tools, channels, subscriptions) |
| `ironclaw_wasm` | WASM sandbox runtime, tool execution |
| `ironclaw_wasm_sandbox_core` | Low-level WASM sandbox integration |
| `ironclaw_wasm_limiter` | Resource limits, memory bounds, timeout enforcement |
| `ironclaw_mcp` | Model Context Protocol server discovery and tunneling |
| `ironclaw_scripts` | Script execution, inline coding (Python, Bash, etc.) |
| `ironclaw_extensions` | Extension lifecycle, manifest discovery, activation |
| `ironclaw_host_runtime` | Host-side effect execution (shell, HTTP, etc.) |
| `ironclaw_processes` | Process sandbox, stdio capture, subprocess management |
| `ironclaw_first_party_extensions` | Built-in tools (GitHub, Google Drive, etc.) |

### 4. Durable State & Events (9 crates)
**Purpose:** Persistence, event sourcing, and state recovery.

| Crate | Purpose |
|-------|---------|
| `ironclaw_events` | Immutable event log, JSONL backend, in-memory store |
| `ironclaw_event_projections` | Snapshot computation, pending gate projection, state cache |
| `ironclaw_event_streams` | Event subscription, filtering, redaction for delivery |
| `ironclaw_reborn_event_store` | Event storage abstraction (PostgreSQL/libSQL adapters) |
| `ironclaw_run_state` | Checkpoint storage, recovery state, progress tracking |
| `ironclaw_threads` | Thread (conversation) lifecycle and metadata |
| `ironclaw_conversations` | Conversation store, trusted inbound, state machine |
| `ironclaw_memory` | Embedding index, semantic search, memory retrieval |
| `ironclaw_memory_native` | Native (on-disk) memory implementation |

### 5. Products & Loops (27 crates)
**Purpose:** Agent loops, product surfaces, and workflows.

| Crate | Purpose |
|-------|---------|
| `ironclaw_agent_loop` | Core agent executor (planning, tool selection, execution, checkpointing) |
| `ironclaw_loop_support` | Utilities for loop implementations |
| `ironclaw_executor` | (v1) Legacy executor; in maintenance mode |
| `ironclaw_turns` | Turn (interaction) state, message sequencing |
| `ironclaw_llm` | LLM provider abstraction (OpenAI, Anthropic, Ollama, etc.) |
| `ironclaw_embeddings` | Embedding provider abstraction (OpenAI, Bedrock, Ollama, etc.) |
| `ironclaw_engine` | (v1) Legacy orchestration; in maintenance mode |
| `ironclaw_reborn` | Reborn runtime kernel and composition |
| `ironclaw_reborn_cli` | Primary CLI/WebUI binary entrypoint |
| `ironclaw_reborn_config` | Config.toml parsing, defaults, resolution |
| `ironclaw_reborn_composition` | Dependency injection, app builder, service wiring |
| `ironclaw_reborn_identity` | User/owner identity, session management |
| `ironclaw_reborn_traces` | Trace recording, replay, structured spans |
| `ironclaw_reborn_openai_compat` | OpenAI-compatible API surface (chat completions, embeddings) |
| `ironclaw_reborn_openai_compat_storage` | PostgreSQL/libSQL adapters for OpenAI API |
| `ironclaw_reborn_webui_ingress` | WebUI HTTP routing, session cookies, CORS |
| `ironclaw_gateway` | (v1) HTTP gateway; mostly ported to Reborn |
| `ironclaw_product_context` | Product-specific request context, user metadata |
| `ironclaw_product_workflow` | Missions, projects, skills, routines, approvals |
| `ironclaw_product_adapters` | Product adapter framework (Slack, Telegram, etc.) |
| `ironclaw_product_adapter_registry` | Discovery and lifecycle of product adapters |
| `ironclaw_wasm_product_adapters` | WASM-based adapter implementations |
| `ironclaw_slack_extension` | Slack workspace adapter |
| `ironclaw_telegram_extension` | Telegram bot adapter |
| `ironclaw_skill_learning` | Skill extraction, classification, and refinement |
| `ironclaw_outbound` | Outbound message delivery (replies, notifications) |
| `ironclaw_triggers` | Trigger system, event subscriptions, automation |

### 6. Storage Backends (8 crates)
**Purpose:** Backend-agnostic persistence with dual support (PostgreSQL + libSQL).

| Crate | Purpose |
|-------|---------|
| `ironclaw_hooks_postgres` | PostgreSQL event hook implementation |
| `ironclaw_hooks_libsql` | libSQL (Turso) event hook implementation |
| `ironclaw_hooks_parity` | Feature parity testing across backends |
| `ironclaw_reborn_event_store` | (Dual-backend) Event store abstraction |
| `ironclaw_run_state` | (Dual-backend) Run state persistence |
| `ironclaw_threads` | (Dual-backend) Thread storage |
| `ironclaw_conversations` | (Dual-backend) Conversation store |
| Other crates | Implement `Db` trait for PostgreSQL and libSQL |

### 7. Utilities & Observability (7 crates)
**Purpose:** Cross-cutting concerns like logging, tracing, and integrations.

| Crate | Purpose |
|-------|---------|
| `ironclaw_observability` | Tracing, metrics, structured logging |
| `ironclaw_skills` | Skill system, skill definitions, extraction |
| `ironclaw_oauth` | OAuth flow management, token refresh |
| `ironclaw_auth` | Authentication, credential types |
| `ironclaw_llm` | (Also in Products) LLM provider abstraction |
| `ironclaw_embeddings` | (Also in Products) Embedding provider abstraction |
| `ironclaw_extractors` | Data extraction utilities |
| `ironclaw_tui` | Terminal UI components (if used) |

## Dependency Flow (Acyclic Upward)

Crate dependencies **flow upward only** — no cycles. The dependency order is:

```
Core Contracts (shared types)
    ↓
Substrates (events, filesystem, memory)
    ↓
Authority & Gates (safety, secrets, approval)
    ↓
Capability Execution (tools, dispatch, WASM)
    ↓
Durable State (events, threads, conversation)
    ↓
Products & Loops (agent, reborn, CLI)
    ↓
Surfaces (WebUI, channels, API)
```

**This ordering ensures:**
- Security decisions (gates, approvals) are isolated from loops
- Loops cannot bypass kernels through imports
- Testing lower layers doesn't require product infrastructure
- Refactoring products doesn't destabilize substrates

## Where to Build New Features

### Decision Tree

```
Is the feature runtime/execution/agent-related?
├─ YES: Goes in crates/ironclaw_reborn* or crates/ironclaw_product*
│  ├─ Agent executor behavior? → ironclaw_agent_loop or ironclaw_executor
│  ├─ Config/composition? → ironclaw_reborn_config or ironclaw_reborn_composition
│  ├─ WebUI/API? → ironclaw_reborn_webui_ingress or ironclaw_gateway
│  ├─ Workflows/missions? → ironclaw_product_workflow
│  └─ New channel (Slack, Discord)? → ironclaw_*_adapter
│
└─ NO: Is it a tool, sandbox, or capability?
   ├─ YES: Goes in ironclaw_capabilities, ironclaw_extensions, ironclaw_wasm, etc.
   │
   └─ NO: Is it a gate (safety, approval, secrets)?
      ├─ YES: Goes in ironclaw_safety, ironclaw_approvals, ironclaw_secrets, etc.
      │
      └─ NO: Is it a substrate (events, storage, filesystem)?
         ├─ YES: Goes in ironclaw_events, ironclaw_filesystem, etc.
         │
         └─ LEGACY v1: Very rarely touch src/. Only maintain existing v1 behavior.
```

### Example Feature Paths

| Feature | Target Crates | Why |
|---------|---------------|-----|
| "Add GitHub issue tool" | `ironclaw_first_party_extensions`, `ironclaw_capabilities` | Capability implementation, registration |
| "Require approval for file writes" | `ironclaw_approvals`, `ironclaw_safety` | Gate logic, policy enforcement |
| "Support Slack threads" | `ironclaw_slack_extension`, `ironclaw_threads` | Channel adapter, thread metadata |
| "Encrypt user files" | `ironclaw_secrets`, `ironclaw_filesystem` | Encryption logic, file storage |
| "Add cost tracking" | `ironclaw_resources`, `ironclaw_events` | Quota system, event projection |

## Architecture Patterns

### Pattern 1: Trait-Based Extensibility

Instead of hardcoding integrations, IronClaw uses traits and registries:

```rust
// Database trait — implement once per backend
pub trait Db: Send + Sync { ... }

// Registered at startup
let db = if postgres_enabled {
    Box::new(PostgresDb::new(...))
} else {
    Box::new(LibSqlDb::new(...))
};

// Loops are database-agnostic
executor.run(db).await
```

**Where to extend:** Add new implementations to the registry in `ironclaw_reborn_composition`.

### Pattern 2: Host Ports (Effect Requests)

Loops don't call the kernel; they request effects through host ports:

```rust
// Loop requests a tool capability
let result = host.request_capability(CapabilityRequest {
    name: "github_issue_create",
    params: {...},
}).await?;

// Kernel gates the request
// (approval? security check? resource limit?)
// then executes if approved
```

**Where to extend:** Add new request types to `ironclaw_host_api`.

### Pattern 3: Event Sourcing

All state changes are immutable events; state is computed from projections:

```
User Action → Event(s) → Event Store
                           ↓
                        Projections
                           ↓
                        (Snapshots, caches, indexes)
                           ↓
                        (Loop queries snapshots, not full log)
```

**Where to extend:** Add event types to `ironclaw_events`, projection logic to `ironclaw_event_projections`.

### Pattern 4: Kernel-Userland Boundary

Every effect request crosses a security checkpoint:

```
Userland (Trustless)  ← Loop requests effect
         ↓ CapabilityRequest
    Kernel (Trusted)   ← Gates check: auth? approval? safety?
         ↓ Effect
   Substrate           ← Durable side effect
```

**Where to extend:** Add gates to `ironclaw_authorization`, `ironclaw_approvals`, `ironclaw_safety`.

## Cross-Crate Communication Patterns

### Event Subscriptions (Decoupled, Asynchronous)
Multiple subsystems listen to durable events:

```
Event Store →
  ├→ Projection System (computes snapshots)
  ├→ Memory Indexer (updates embeddings)
  ├→ Audit Logger (logs for security)
  └→ Trigger System (fires automations)
```

**Implementation:** Use `ironclaw_event_streams` for subscription.

### Host Ports (Loop-to-Kernel, Synchronous)
Loops request capabilities through ports; kernel enforces policy:

```
Loop → CapabilityRequest → Kernel Ports → Policy Checks → Execution
```

**Implementation:** Define request/response types in `ironclaw_host_api`, handle in `ironclaw_capabilities`.

### Trait Objects (Polymorphism)
Different implementations of the same behavior, selected at startup:

```
LlmProvider (Anthropic | OpenAI | Ollama | ...)
DbBackend (PostgreSQL | libSQL)
MemoryStore (Native | Bedrock | Pinecone | ...)
```

**Implementation:** Define trait in a core crate, register implementations at startup.

## Key Architectural Decisions

1. **Secrets never inline** — environment variable names only; actual secrets in env
2. **Loops are untrusted** — all loop requests pass through kernel gates
3. **Approval leases are exact-invocation scoped** — blanket approval is not possible
4. **Prompt templates live in files** — not hardcoded in Rust (allows rapid iteration)
5. **Event sourcing is immutable** — all state computable from events
6. **Backwards compatibility in event schema** — events are versioned and must be readable forever
7. **Dependency acyclic** — no circular imports; upward flow only
8. **Active-thread lock prevents duplicate work** — only one loop runs per thread at a time
9. **LoopExit is a claim, not trusted state** — kernel still checks permissions even if loop says "done"
10. **Large prompts in files, not code** — enables A/B testing and rapid iteration without rebuilding
11. **Logging discipline for REPL/TUI** — use `debug!()` not `info!()` to avoid spamming user output
12. **Test-first discipline** — every bug fix includes a regression test

## See Also

- **[Crate Reference](crates.md)** — Detailed breakdown of all 68+ crates
- **[Data Model](data-model.md)** — Events, threads, turns, capabilities
- **[Security & Safety](security.md)** — Kernel boundary, threat model, approval gates
- **[AGENTS.md](/AGENTS.md)** — Quick rules and code discovery
- **[CLAUDE.md](/CLAUDE.md)** — Subsystem deep-dives by crate/module

---

**Last updated:** Auto-generated by OpenWiki. For corrections, file a PR.
