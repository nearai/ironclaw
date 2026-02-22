# IronClaw — Master Architecture Document

> Updated: 2026-02-22 (v0.9.0) | Comprehensive reference for contributors

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [System Architecture Diagram](#2-system-architecture-diagram)
3. [Module Catalog](#3-module-catalog)
4. [Dependency Graph](#4-dependency-graph)
5. [Data Flow: Message Lifecycle](#5-data-flow-message-lifecycle)
6. [Data Flow: Tool Execution](#6-data-flow-tool-execution)
7. [Data Flow: WASM Tool Build](#7-data-flow-wasm-tool-build)
8. [Configuration Architecture](#8-configuration-architecture)
9. [Security Model](#9-security-model)
10. [Storage and Persistence](#10-storage-and-persistence)
11. [Key Design Patterns](#11-key-design-patterns)
12. [Cross-Module Statistics](#12-cross-module-statistics)

---

## 1. Executive Summary

IronClaw is a secure personal AI assistant written in Rust, developed under the NEAR AI project. It is designed around a single principle: the assistant works for the user, not for a platform. All conversation data is stored locally or in a user-controlled database, credentials are encrypted at rest using AES-256-GCM with HKDF-derived per-secret keys, and untrusted code runs inside a WASM sandbox with capability-based permissions.

The binary is a single self-contained executable. There is no separate daemon manager, no sidecar service, and no runtime dependency on a cloud control plane. The agent initializes all subsystems at startup through a five-phase `AppBuilder` sequence: database connection and schema migration, secrets store creation and LLM key injection, LLM provider chain construction with failover and circuit-breaking wrappers, tool and workspace initialization, and finally extension loading (WASM tools, MCP servers, skill registry). Background subsystems — heartbeat, routine engine, self-repair, session pruning — run as tokio tasks within the same process.

IronClaw differs from TypeScript-based AI gateways in several important ways. First, it compiles to a native binary with no Node.js or Python runtime requirement. Second, it uses libsql (an embedded SQLite fork by Turso) or PostgreSQL as its storage layer, both exposed through a single `Database` trait abstraction with approximately sixty async methods. Third, multi-LLM support is provided by `rig-core`, a Rust framework that abstracts over OpenAI, Anthropic, Ollama, and OpenAI-compatible endpoints. NEAR AI is supported natively through a custom `NearAiProvider` that uses the Responses API with session-token authentication and response chaining for efficient multi-turn conversations. Tinfoil private inference, which runs models in hardware-attested TEEs, is also supported via the OpenAI-compatible Chat Completions API.

The channel abstraction is the core extensibility point for message ingestion. A `Channel` trait with five lifecycle methods (`start`, `respond`, `send_status`, `broadcast`, `health_check`) allows any input source to feed the same agent loop. Implemented channels include an interactive REPL channel (rustyline + termimad), an HTTP webhook server, a web gateway with SSE/WebSocket streaming and a single-page browser UI, and a WASM channel runtime that loads compiled channel implementations at startup. A `ChannelManager` merges all active streams via `futures::stream::select_all` and provides a single injection sender for background tasks to push synthetic messages into the agent loop without being full Channel implementations.

---

## 2. System Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                               CHANNELS LAYER                                    │
│                                                                                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐   │
│  │ REPL Channel │  │ HTTP Webhook │  │ Web Gateway  │  │  WASM Channels   │   │
│  │ (rustyline)  │  │  (axum)      │  │ SSE/WebSocket│  │(Tg,Slack,WA,Dc)  │   │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └──────┬───────────┘   │
│         │                 │                  │                  │               │
│         └─────────────────┴──────────────────┴──────────────────┘               │
│                                      │                                          │
│                          ChannelManager::start_all()                            │
│                        futures::stream::select_all()                            │
│                             + inject_rx (background)                            │
└──────────────────────────────────────┬──────────────────────────────────────────┘
                                       │ IncomingMessage stream
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────────┐
│                              AGENT CORE                                         │
│                                                                                 │
│  ┌─────────────────────────────────────────────────────────────────────────┐   │
│  │                        Agent::run() event loop                          │   │
│  │                                                                         │   │
│  │  ┌────────────┐  ┌───────────────┐  ┌────────────┐  ┌───────────────┐  │   │
│  │  │ Submission │  │ SessionManager│  │  Router    │  │HookRegistry   │  │   │
│  │  │  Parser    │  │ thread model  │  │  (intent   │  │(Inbound/Out-  │  │   │
│  │  │ undo/redo  │  │ + state machine│  │ classif.)  │  │ bound hooks)  │  │   │
│  │  └────────────┘  └───────────────┘  └────────────┘  └───────────────┘  │   │
│  │                                                                         │   │
│  │  ┌────────────┐  ┌───────────────┐  ┌────────────┐  ┌───────────────┐  │   │
│  │  │ Scheduler  │  │ContextManager │  │  Compaction│  │  CostGuard    │  │   │
│  │  │ (parallel  │  │ (job isolation│  │  (context  │  │(daily budget, │  │   │
│  │  │  jobs)     │  │  + state mach)│  │  window)   │  │ hourly rate)  │  │   │
│  │  └────────────┘  └───────────────┘  └────────────┘  └───────────────┘  │   │
│  └─────────────────────────────────────────────────────────────────────────┘   │
│                                                                                 │
│  Background tasks (tokio::spawn):                                               │
│  ┌──────────────────────────────────────────────────────────────────────────┐  │
│  │  HeartbeatRunner │ RoutineEngine (cron + events) │ SelfRepair │ Pruning  │  │
│  └──────────────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────┬──────────────────────────────────────────┘
                                       │ CompletionRequest
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────────┐
│                              LLM BACKEND                                        │
│                                                                                 │
│  ┌──────────────────────────────────────────────────────────────────────────┐  │
│  │                    Arc<dyn LlmProvider> chain                            │  │
│  │                                                                          │  │
│  │  ┌──────────────┐   ┌──────────────────┐   ┌──────────────────────────┐ │  │
│  │  │CachedProvider│──▶│CircuitBreaker    │──▶│ FailoverProvider         │ │  │
│  │  │(response TTL)│   │(failure threshold│   │ (primary + fallback)     │ │  │
│  │  └──────────────┘   └──────────────────┘   └──────────────┬───────────┘ │  │
│  │                                                            │             │  │
│  │  ┌─────────────────────────────────────────────────────────▼──────────┐ │  │
│  │  │         Concrete Providers (wrapped by RigAdapter)                  │ │  │
│  │  │  NearAiProvider │ NearAiChatProvider │ RigAdapter (OpenAI/Anthropic │ │  │
│  │  │  Ollama / Tinfoil / OpenAI-compatible)                              │ │  │
│  │  └─────────────────────────────────────────────────────────────────────┘ │  │
│  └──────────────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────┬──────────────────────────────────────────┘
                                       │ tool_calls in CompletionResponse
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────────┐
│                           TOOL EXECUTION LAYER                                  │
│                                                                                 │
│  ┌──────────────────────────────────────────────────────────────────────────┐  │
│  │                         ToolRegistry                                     │  │
│  │            Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>                   │  │
│  │                                                                          │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌────────────┐  │  │
│  │  │  Built-in    │  │  WASM Tools  │  │  MCP Client  │  │  Builder   │  │  │
│  │  │  (echo, http │  │  (wasmtime,  │  │  (JSON-RPC   │  │  Tool      │  │  │
│  │  │  shell, file,│  │  component   │  │  over HTTP)  │  │  (LLM-     │  │  │
│  │  │  memory, job)│  │  model)      │  │              │  │  driven)   │  │  │
│  │  └──────────────┘  └──────────────┘  └──────────────┘  └────────────┘  │  │
│  └──────────────────────────────────────────────────────────────────────────┘  │
│                                                                                 │
│  Safety Layer (runs on every tool output):                                      │
│  ┌──────────────────────────────────────────────────────────────────────────┐  │
│  │  LeakDetector → Sanitizer → Policy → wrap_for_llm(<tool_output> tags)   │  │
│  └──────────────────────────────────────────────────────────────────────────┘  │
│                                                                                 │
│  Sandbox (Docker, for container-domain tools):                                  │
│  ┌──────────────────────────────────────────────────────────────────────────┐  │
│  │  ContainerJobManager (bollard) │ NetworkProxy (hyper) │ CredentialInject │  │
│  └──────────────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────┬──────────────────────────────────────────┘
                                       │ ToolOutput → ToolResult
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────────┐
│                            STORAGE LAYER                                        │
│                                                                                 │
│  ┌──────────────────────────────┐   ┌──────────────────────────────────────┐   │
│  │  Arc<dyn Database>           │   │  Workspace (memory filesystem)       │   │
│  │  ┌──────────┐  ┌──────────┐  │   │  ┌──────────────┐  ┌──────────────┐ │   │
│  │  │PostgreSQL│  │  libSQL  │  │   │  │ memory_docs  │  │ memory_chunks│ │   │
│  │  │(postgres │  │(libsql   │  │   │  │ (path-based) │  │(FTS5+vector) │ │   │
│  │  │ feature) │  │ feature) │  │   │  └──────────────┘  └──────────────┘ │   │
│  │  └──────────┘  └──────────┘  │   │  Hybrid search: BM25 + RRF          │   │
│  │  ~60 async methods            │   └──────────────────────────────────────┘   │
│  │  refinery migrations          │                                              │
│  └──────────────────────────────┘   ┌──────────────────────────────────────┐   │
│                                     │  SecretsStore (AES-256-GCM)          │   │
│  ┌──────────────────────────────┐   │  OS Keychain (master key)            │   │
│  │  SkillRegistry               │   │  per-secret HKDF-SHA256 derivation   │   │
│  │  ~/.ironclaw/skills/         │   └──────────────────────────────────────┘   │
│  │  ClawHub catalog client      │                                              │
│  └──────────────────────────────┘                                              │
└─────────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Module Catalog

The following table lists every source module directory and the key top-level files in `src/`.

| Module | Path | Purpose |
|--------|------|---------|
| `agent` | `src/agent/` | Core agent orchestration: main event loop, session management, job scheduling, self-repair, heartbeat, routine engine, context compaction, undo/redo, skill selection, cost guardrails |
| `channels` | `src/channels/` | Multi-channel input abstraction: `Channel` trait, `ChannelManager` (stream merge), HTTP webhook, web gateway (axum + SSE + WebSocket), WASM channel runtime, REPL |
| `cli` | `src/cli/` | CLI command surface: onboarding, config, tool, mcp, memory, pairing, service, doctor, status |
| `config` | `src/config/` | Configuration loading from environment, DB settings table, and optional TOML overlay. Sub-modules per domain: agent, builder, channels, database, embeddings, heartbeat, llm, routines, safety, sandbox, secrets, skills, tunnel, wasm |
| `context` | `src/context/` | Per-job state isolation: `JobState` state machine (Pending → InProgress → Completed/Failed/Stuck), `JobContext`, `ContextManager` for concurrent job tracking |
| `db` | `src/db/` | Database abstraction: `Database` trait (~60 async methods), PostgreSQL backend (`deadpool-postgres`, `refinery` migrations), libSQL/Turso embedded backend |
| `estimation` | `src/estimation/` | Cost and time estimation with exponential moving average learning: `CostEstimator`, `TimeEstimator`, `ValueEstimator` |
| `evaluation` | `src/evaluation/` | Success evaluation: `SuccessEvaluator` trait, rule-based and LLM-based evaluators, `MetricsCollector` |
| `extensions` | `src/extensions/` | `ExtensionManager`: coordinates MCP server auth and activation, WASM tool install/remove, registers in-chat discovery tools |
| `history` | `src/history/` | Persistence for conversation threads and analytics: PostgreSQL repositories, aggregation queries (JobStats, ToolStats) |
| `hooks` | `src/hooks/` | `HookRegistry` for Inbound/Outbound message interception: hooks can modify, reject, or pass through messages at the agent loop boundary |
| `llm` | `src/llm/` | LLM provider chain: `LlmProvider` trait, `NearAiChatProvider` (Chat Completions, dual auth: session token + API key), `SmartRoutingProvider` (routes Simple/Moderate/Complex requests to cheap vs primary model), `RigAdapter` (rig-core bridge for OpenAI/Anthropic/Ollama/Tinfoil), `FailoverProvider`, `CircuitBreakerProvider`, `CachedProvider`, `RetryProvider`, session token management |
| `observability` | `src/observability/` | Tracing and metrics backend configuration |
| `orchestrator` | `src/orchestrator/` | Internal HTTP API served to Docker sandbox containers: LLM proxy endpoint, job event streaming, per-job bearer token auth, `ContainerJobManager` (bollard lifecycle) |
| `pairing` | `src/pairing/` | Device pairing and authentication helpers for remote channel setup |
| `registry` | `src/registry/` | Extension/tool registry client for discovering installable tools and channels |
| `safety` | `src/safety/` | Prompt injection defense: `Sanitizer` (pattern detection, XML escaping), `Validator` (length, encoding checks), `Policy` (rule-based actions: Block/Warn/Review/Sanitize), `LeakDetector` (15+ secret patterns with Block/Redact/Warn actions), `CredentialDetector` (HTTP param credential detection: requires approval when auth data is present in headers/URL) |
| `sandbox` | `src/sandbox/` | Docker-based job isolation: `SandboxManager`, `ContainerRunner`, `NetworkProxy` (hyper HTTP/CONNECT proxy with domain allowlist and credential injection), `SandboxPolicy` (ReadOnly/WorkspaceWrite/FullAccess) |
| `secrets` | `src/secrets/` | Encrypted credential storage: AES-256-GCM encryption, HKDF-SHA256 per-secret key derivation, PostgreSQL and libSQL backends, OS keychain integration (macOS: security-framework, Linux: secret-service/KWallet) |
| `setup` | `src/setup/` | 7-step interactive onboarding wizard: database backend selection, NEAR AI authentication, secrets master key setup, channel configuration |
| `skills` | `src/skills/` | SKILL.md prompt extension system: `SkillRegistry` (discover, install, remove), deterministic scorer (keywords/tags/regex), `SkillTrust` model (Trusted vs Installed), tool attenuation (trust-based ceiling), gating requirements (bins/env/config), `SkillCatalog` (ClawHub HTTP client) |
| `tools` | `src/tools/` | Extensible tool system: `Tool` trait, `ToolRegistry` (shadowing protection for built-in names), built-in tools (echo, time, json, http, shell, file ops, memory, job mgmt, routines, extensions, skills, `HtmlConverter` (HTML-to-Markdown, two-stage: readability extraction + markdown conversion; feature-gated `html-to-markdown`)), WASM sandbox (wasmtime component model, fuel metering, memory limits), MCP client (JSON-RPC over HTTP), dynamic software builder |
| `tunnel` | `src/tunnel/` | Tunnel/ngrok-style public URL provisioning for webhook channels |
| `worker` | `src/worker/` | Runs inside Docker containers: `Worker` execution loop, tool calls via LLM reasoning, Claude Code bridge (spawns `claude` CLI), orchestrator HTTP client, proxy LLM provider that forwards requests through orchestrator |
| `workspace` | `src/workspace/` | Persistent memory (OpenClaw-inspired): path-based document store, content chunking (800 tokens, 15% overlap), `EmbeddingProvider` trait (OpenAI, NEAR AI, Ollama), hybrid FTS+vector search via Reciprocal Rank Fusion, identity file injection into system prompt, heartbeat checklist |
| `app.rs` | `src/app.rs` | `AppBuilder`: five-phase initialization sequence producing `AppComponents` (all shared state for channel wiring and agent construction) |
| `bootstrap.rs` | `src/bootstrap.rs` | Chicken-and-egg bootstrap: loads `~/.ironclaw/.env` before database connects, one-time migration from legacy `settings.json` and `bootstrap.json` formats |
| `service.rs` | `src/service.rs` | OS service management: generates launchd plist (macOS) or systemd user unit (Linux), handles install/start/stop/status/uninstall lifecycle |
| `main.rs` | `src/main.rs` | Entry point: clap CLI dispatch, startup sequencing (dotenvy, bootstrap, config, AppBuilder, channel wiring, Agent::run) |
| `error.rs` | `src/error.rs` | Crate-wide error types using `thiserror` |
| `settings.rs` | `src/settings.rs` | `Settings` struct for disk and DB key-value settings with TOML overlay support |

---

## 4. Dependency Graph

The following diagram shows which modules depend on which. Arrows point from dependent to dependency (A → B means "A imports B").

```
src/main.rs
    │
    ├──▶ config (Config::from_env)
    │        └──▶ bootstrap (dotenvy, ~/.ironclaw/.env)
    │        └──▶ settings (Settings::load, TOML overlay)
    │
    ├──▶ app (AppBuilder)
    │        ├──▶ db (Database trait)
    │        │       ├──▶ db::postgres (deadpool-postgres, refinery)
    │        │       └──▶ db::libsql  (libsql crate, in-process migrations)
    │        │
    │        ├──▶ secrets (SecretsStore)
    │        │       ├──▶ secrets::crypto  (aes-gcm, hkdf, sha2)
    │        │       ├──▶ secrets::keychain (security-framework / secret-service)
    │        │       └──▶ db (storage backend)
    │        │
    │        ├──▶ llm (create_llm_provider)
    │        │       ├──▶ llm::nearai_chat   (NearAiChatProvider, dual auth: session token + API key)
    │        │       ├──▶ llm::smart_routing (SmartRoutingProvider, cheap vs primary cascade)
    │        │       ├──▶ llm::rig_adapter   (rig-core: OpenAI/Anthropic/Ollama/Tinfoil)
    │        │       ├──▶ llm::failover      (FailoverProvider)
    │        │       ├──▶ llm::circuit_breaker
    │        │       ├──▶ llm::response_cache
    │        │       └──▶ llm::session       (SessionManager, token renewal)
    │        │
    │        ├──▶ safety (SafetyLayer)
    │        │       ├──▶ safety::sanitizer
    │        │       ├──▶ safety::validator
    │        │       ├──▶ safety::policy
    │        │       ├──▶ safety::leak_detector
    │        │       └──▶ safety::credential_detect (HTTP param credential detection)
    │        │
    │        ├──▶ tools (ToolRegistry)
    │        │       ├──▶ tools::builtin     (echo, time, json, http, shell, file, memory, job, html_converter)
    │        │       ├──▶ tools::wasm        (wasmtime, WasmToolRuntime, WasmToolWrapper)
    │        │       │       ├──▶ secrets    (credential injection at host boundary)
    │        │       │       └──▶ safety     (leak detection on WASM output)
    │        │       ├──▶ tools::mcp         (McpClient, JSON-RPC over HTTP)
    │        │       └──▶ tools::builder     (LlmSoftwareBuilder, WASM generation)
    │        │               └──▶ llm        (LLM-driven iterative build loop)
    │        │
    │        ├──▶ workspace (Workspace)
    │        │       ├──▶ db                 (WorkspaceStorage::Db)
    │        │       ├──▶ workspace::chunker
    │        │       ├──▶ workspace::embeddings (EmbeddingProvider)
    │        │       └──▶ workspace::search  (RRF)
    │        │
    │        ├──▶ extensions (ExtensionManager)
    │        │       ├──▶ tools::mcp
    │        │       ├──▶ tools::wasm
    │        │       ├──▶ secrets
    │        │       └──▶ hooks
    │        │
    │        └──▶ skills (SkillRegistry, SkillCatalog)
    │                ├──▶ skills::parser
    │                ├──▶ skills::selector
    │                ├──▶ skills::attenuation
    │                ├──▶ skills::gating
    │                └──▶ skills::catalog    (ClawHub HTTP client)
    │
    ├──▶ channels (ChannelManager)
    │        ├──▶ channels::web      (GatewayChannel, axum, SSE, WebSocket)
    │        │       └──▶ channels::web::auth  (Bearer token, constant-time compare)
    │        ├──▶ channels::http     (HttpChannel, axum webhook)
    │        ├──▶ channels::repl     (ReplChannel, rustyline)
    │        ├──▶ channels::wasm     (WASM channel runtime; loads channels-src/: Telegram, Slack, Discord, WhatsApp)
    │        └──▶ cli                 (clap command routing)
    │
    └──▶ agent (Agent)
             ├──▶ agent::agent_loop       (main run loop, message dispatch)
             ├──▶ agent::session_manager  (Session, Thread, Turn, PendingApproval)
             ├──▶ agent::scheduler        (parallel job execution)
             ├──▶ agent::worker           (per-job LLM reasoning loop)
             ├──▶ agent::dispatcher       (tool dispatch, agentic loop)
             ├──▶ agent::self_repair      (stuck job detection and recovery)
             ├──▶ agent::heartbeat        (periodic workspace checklist execution)
             ├──▶ agent::routine_engine   (cron ticker + event trigger matching)
             ├──▶ agent::compaction       (context window management)
             ├──▶ agent::cost_guard       (daily budget enforcement)
             ├──▶ llm
             ├──▶ tools
             ├──▶ safety
             ├──▶ workspace
             ├──▶ db
             ├──▶ channels
             ├──▶ context
             ├──▶ hooks
             └──▶ skills

orchestrator (separate runtime inside Docker host process)
    ├──▶ llm        (LLM proxy endpoint for worker)
    ├──▶ db         (job event storage)
    └──▶ sandbox    (ContainerJobManager)

worker (runs inside Docker container, connects to orchestrator over HTTP)
    ├──▶ worker::proxy_llm   (forwards LLM calls through orchestrator API)
    ├──▶ worker::claude_bridge (spawns claude CLI for Claude Code mode)
    ├──▶ tools               (container-domain tools: shell, file, apply_patch)
    └──▶ worker::api         (orchestrator HTTP client)
```

---

## 5. Data Flow: Message Lifecycle

This section traces the path of a user message from arrival to final response, using the web gateway as the concrete example.

**Step 1 — HTTP POST received by axum router**

The web gateway (`src/channels/web/server.rs`) registers approximately forty API endpoints. An incoming chat message arrives as `POST /api/chat/send`. The axum handler extracts the request body and the `Authorization` header.

**Step 2 — Bearer token authentication**

The `auth` middleware (`src/channels/web/auth.rs`) performs a timing-safe comparison (using the `subtle` crate) between the provided token and the configured `GATEWAY_AUTH_TOKEN`. Requests with missing or incorrect tokens receive a 401 response immediately.

**Step 3 — IncomingMessage construction and dispatch**

The handler constructs an `IncomingMessage` value (UUID, channel name `"web"`, user ID from config, content string, optional thread ID from the JSON body, received timestamp, JSON metadata). This message is pushed into the channel's internal `mpsc` sender, which feeds the merged stream in the `ChannelManager`.

**Step 4 — Agent event loop receives the message**

`Agent::run()` polls `message_stream.next()` inside a `tokio::select!`. The `IncomingMessage` is dequeued and passed to `handle_message()`.

**Step 5 — Hook interception (BeforeInbound)**

`HookRegistry::run()` is called with a `HookEvent::Inbound`. Hooks can reject the message (returns an error string to the user), modify the content (mutation is reflected in the submission parser), or pass through unchanged.

**Step 6 — Submission parsing**

`SubmissionParser::parse()` examines the message content for control prefixes: `/undo`, `/redo`, `/compact`, `/clear`, `/quit`, `/thread`, `approve:`, etc. A plain message becomes `Submission::UserInput { content }`.

**Step 7 — Thread hydration and session resolution**

If the message carries an external thread ID, the session manager checks whether the thread is already in memory. If not, it rehydrates from the database (loading conversation turns). `SessionManager::resolve_thread()` returns a `(Arc<Mutex<Session>>, thread_id)` pair.

**Step 8 — Auth-mode interception**

If the thread has a `PendingAuth` state (awaiting an OAuth token or manual API key entry), the raw message content is routed directly to credential storage via `process_auth_token()`. Normal processing is bypassed entirely.

**Step 9 — LLM context construction**

`process_user_input()` builds the completion request. This includes:

- The system prompt from identity files (`AGENTS.md`, `SOUL.md`, `USER.md`, `IDENTITY.md`) loaded from the workspace
- Active skills selected by `select_active_skills()` using deterministic keyword/tag/regex scoring, injected as `<skill name="..." trust="...">` blocks
- Conversation history for the current thread (all prior turns)
- The user's new message

**Step 10 — Skill selection and tool attenuation**

`prefilter_skills()` scores all loaded skills against the message content and selects those that fit within the `SKILLS_MAX_CONTEXT_TOKENS` budget. If any `Installed`-trust skills are active, `attenuate_tools()` restricts the tool set to read-only tools for the entire turn, preventing privilege escalation.

**Step 11 — LLM API call**

The agent calls `llm.complete(request)` on the `Arc<dyn LlmProvider>` chain. The chain applies response caching, circuit-breaking, failover, and retry as configured. The concrete provider serializes the request (NEAR AI Responses API for session-based auth, or Chat Completions for API-key mode) and makes the HTTP call via `reqwest`.

**Step 12 — Tool call extraction**

If the LLM response includes tool calls (`finish_reason: tool_calls`), the agent's dispatcher extracts each `ToolCall` and routes it to the `ToolRegistry`. The agentic loop continues making LLM calls until the model produces a final text response with no tool calls or the turn budget is exhausted.

**Step 13 — Response streaming**

The web gateway sends incremental status updates via an SSE broadcast channel (`src/channels/web/sse.rs`) as the agent processes. `StatusUpdate::StreamChunk(text)` events are sent for streaming output. `StatusUpdate::ToolStarted` and `StatusUpdate::ToolCompleted` events update the UI's activity indicators.

**Step 14 — Hook interception (BeforeOutbound)**

Before the final response string is sent back, `HookRegistry::run()` is called with a `HookEvent::Outbound`. Hooks can modify the content or suppress delivery entirely.

**Step 15 — Response delivery and history persistence**

`ChannelManager::respond()` routes the `OutgoingResponse` to the originating channel. The response and the full turn (user message, assistant response, tool calls, tool results) are persisted to the `conversations` and `agent_jobs` tables in the database.

---

## 6. Data Flow: Tool Execution

This section details what happens between the LLM emitting a tool call and the tool result being fed back into the next LLM turn.

**Step 1 — Tool call parsed from LLM response**

The `LlmProvider::complete()` return value includes a `Vec<ToolCall>`, each containing a tool name and a `serde_json::Value` parameters object.

**Step 2 — Tool lookup in registry**

`ToolRegistry::get(name)` acquires a read lock on the inner `HashMap<String, Arc<dyn Tool>>` and clones the `Arc`. If the tool name is not registered, the agent returns an error result to the LLM explaining the tool is unavailable.

**Step 3 — Approval gate**

If `tool.requires_approval()` returns `true`, or if `tool.requires_approval_for(&params)` returns `true` for this specific invocation (e.g., destructive shell commands), the agent suspends execution and emits a `StatusUpdate::ApprovalNeeded` event. The channel delivers an approval prompt to the user (inline card in the web UI, formatted text in REPL). The thread enters `PendingApproval` state. The tool does not execute until the user confirms or denies.

**Step 4 — Domain routing**

`tool.domain()` returns either `ToolDomain::Orchestrator` (safe to run in the main process) or `ToolDomain::Container` (must run inside a sandboxed Docker container). Container-domain tools include shell execution, file operations, and code editing. Orchestrator-domain tools include echo, time, JSON operations, HTTP calls, memory access, and job management.

For `ToolDomain::Container` tools when sandbox is enabled, the `create_job` tool is used to launch a Docker container via `ContainerJobManager`. The `Worker` process inside the container connects to the `Orchestrator` HTTP API to receive tool calls and proxy LLM requests.

**Step 5 — Safety pre-check**

`SafetyLayer::validate_input()` is called on the serialized parameters before execution. If the validator rejects the input (e.g., excessively long content, forbidden byte patterns), an error is returned without invoking the tool.

**Step 6 — Tool execution with timeout**

`tool.execute(params, &job_context)` is called. An outer `tokio::time::timeout` wrapper enforces `tool.execution_timeout()` (default 60 seconds, overridable per tool). If execution times out, `ToolError::Timeout` is returned.

**Step 7 — WASM execution path (for WASM tools)**

For tools registered as `WasmToolWrapper`, execution proceeds through `WasmToolRuntime`:

- The compiled WASM component is instantiated via `wasmtime`
- Fuel metering enforces compute limits (the module is killed if it exhausts its fuel budget)
- Memory limits enforce the maximum linear memory the module can allocate
- Host functions provide logging, current time, and workspace read access
- HTTP calls from WASM pass through the `allowlist` module before being forwarded; the `CredentialInjector` injects secrets into request headers so the WASM module never sees raw credential values
- The WASM module's return value is deserialized as a `ToolOutput`

**Step 8 — Safety post-processing**

If `tool.requires_sanitization()` is `true` (the default for all external-facing tools), `SafetyLayer::sanitize_tool_output(tool_name, output_string)` is called:

  1. Length check: truncates if the output exceeds `max_output_length`
  2. `LeakDetector::scan_and_clean()`: scans for 15+ secret patterns (API keys, connection strings, private keys) and redacts or blocks as configured per pattern
  3. `Policy::check()`: applies policy rules; `Block` violations replace the entire output, `Sanitize` violations force sanitizer pass
  4. `Sanitizer::sanitize()`: detects injection patterns, escapes dangerous content

**Step 9 — Wrapping for LLM**

`SafetyLayer::wrap_for_llm(tool_name, content, sanitized)` wraps the output in XML delimiters that create a structural boundary between trusted instructions and untrusted tool data:

```xml
<tool_output name="shell" sanitized="true">
[escaped content here]
</tool_output>
```

**Step 10 — Tool result fed back**

The wrapped output is attached as a `ToolResult` in the next `CompletionRequest`. The conversation history for this turn grows: user message → assistant (tool calls) → tool results → assistant (final response).

---

## 7. Data Flow: WASM Tool Build

The `build_software` tool (registered when `BUILDER_ENABLED=true` and `allow_local_tools=true` or sandbox is disabled) allows the LLM to dynamically create new WASM tools at runtime.

**Step 1 — User requests a new capability**

The user describes the tool they need in natural language. The LLM identifies this as a tool-building task and calls `build_software` with a natural language description of the desired capability, input/output schema, and optional target language (Rust is the default for WASM targets).

**Step 2 — BuildSoftwareTool dispatches to LlmSoftwareBuilder**

`BuildSoftwareTool::execute()` passes the parameters to `LlmSoftwareBuilder`, which orchestrates the full build loop. The builder has access to shell, file read/write, and list-directory tools for interacting with the build environment.

**Step 3 — Requirements analysis**

The builder makes an LLM call to analyze the request and produce a `BuildRequirement`: the target `SoftwareType` (WasmTool, CliBinary, Script), the implementation `Language`, input/output field definitions, and a list of external dependencies.

**Step 4 — Project scaffolding**

`TemplateEngine::scaffold()` generates a project directory from templates: a `Cargo.toml` with the appropriate WASM component model dependencies, a `lib.rs` stub with the correct WIT interface, and a `wit/` directory with the interface definition. For non-Rust targets, analogous scaffolding is generated.

**Step 5 — Iterative LLM code generation**

The builder enters an iterative loop:

  1. LLM generates or modifies the implementation source code
  2. The shell tool runs `cargo build --target wasm32-wasip2 --release`
  3. If compilation fails, error output is fed back into the next LLM turn
  4. The loop continues until compilation succeeds or the maximum iteration count is reached

**Step 6 — Test harness execution**

`TestHarness::run()` executes any provided test cases against the compiled WASM module. Test failures are fed back to the LLM for correction using the same iterative loop pattern.

**Step 7 — WASM validation**

`WasmValidator::validate()` uses `wasmparser` to verify:

- The binary is a valid WASM module or component
- The component model interface matches the expected WIT definition
- The module does not import any host functions beyond the allowed set

**Step 8 — Registration**

On success, `ToolRegistry::register_wasm()` compiles the module via `WasmToolRuntime::prepare()`, wraps it in a `WasmToolWrapper`, and registers it under the tool name. The WASM binary is also persisted to the `wasm_tools` database table for survival across restarts.

**Step 9 — Capability grant (pending UX)**

Newly built tools receive empty capabilities by default (no HTTP access, no secret access). The user must explicitly grant network endpoints and secrets access through the extension management tools (`tool_auth`). This is a known limitation pending UX work.

---

## 8. Configuration Architecture

IronClaw uses a layered configuration system with the following priority order, from highest to lowest:

```
1. Explicit environment variables (already set in the process environment)
2. Variables from ./.env  (loaded via dotenvy::dotenv())
3. Variables from ~/.ironclaw/.env  (loaded via bootstrap::load_ironclaw_env())
4. TOML config file overlay (~/.ironclaw/config.toml or --config path)
5. Database settings table (per-user key-value store)
6. Compiled-in defaults
```

The "chicken-and-egg" bootstrap problem — needing the database URL before connecting to the database — is solved by `bootstrap.rs`. The `DATABASE_URL` (and related bootstrap variables like `DATABASE_BACKEND`, `LIBSQL_PATH`) must be available in the environment or in `~/.ironclaw/.env` before any other initialization. Everything else (LLM settings, agent behavior, channel configuration) can be stored in the database and reloaded after connection.

**Config loading sequence in AppBuilder:**

```
1. dotenvy::dotenv()             — loads ./.env
2. bootstrap::load_ironclaw_env() — loads ~/.ironclaw/.env
3. Config::from_env()            — builds initial config from env vars + settings.json fallback
4. AppBuilder::init_database()   — connects to DB, runs migrations
5. bootstrap::migrate_disk_to_db() — one-time migration of legacy disk settings
6. Config::from_db_with_toml()   — reloads config from DB settings table + TOML overlay
7. AppBuilder::init_secrets()    — initializes secrets store
8. inject_llm_keys_from_secrets() — injects encrypted API keys into INJECTED_VARS overlay
9. Config::from_db_with_toml()   — second reload to pick up newly injected keys
```

**Config sub-modules and their primary env vars:**

| Sub-module | Key env vars |
|------------|-------------|
| `config::database` | `DATABASE_BACKEND`, `DATABASE_URL`, `LIBSQL_PATH`, `LIBSQL_URL`, `LIBSQL_AUTH_TOKEN` |
| `config::llm` | `LLM_BACKEND`, `NEARAI_API_KEY`, `NEARAI_MODEL`, `NEARAI_BASE_URL`, `NEARAI_SESSION_PATH`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `OLLAMA_BASE_URL`, `LLM_BASE_URL`, `LLM_API_KEY`, `LLM_MODEL`, `TINFOIL_API_KEY` |
| `config::agent` | `AGENT_NAME`, `AGENT_MAX_PARALLEL_JOBS`, `MAX_COST_PER_DAY_CENTS`, `MAX_ACTIONS_PER_HOUR` |
| `config::sandbox` | `SANDBOX_ENABLED`, `SANDBOX_IMAGE`, `SANDBOX_MEMORY_LIMIT_MB`, `SANDBOX_TIMEOUT_SECS`, `SANDBOX_POLICY`, `SANDBOX_CPU_SHARES` |
| `config::channels` | `GATEWAY_ENABLED`, `GATEWAY_HOST`, `GATEWAY_PORT`, `GATEWAY_AUTH_TOKEN`, `HTTP_PORT`, `HTTP_HOST`, `HTTP_WEBHOOK_SECRET` |
| `config::safety` | `SAFETY_MAX_OUTPUT_LENGTH`, `SAFETY_INJECTION_CHECK_ENABLED` |
| `config::wasm` | `WASM_ENABLED`, `WASM_TOOLS_DIR` |
| `config::secrets` | `SECRETS_MASTER_KEY` (or OS keychain) |
| `config::heartbeat` | `HEARTBEAT_ENABLED`, `HEARTBEAT_INTERVAL_SECS`, `HEARTBEAT_NOTIFY_CHANNEL` |
| `config::skills` | `SKILLS_ENABLED`, `SKILLS_MAX_CONTEXT_TOKENS`, `SKILLS_MAX_ACTIVE`, `SKILLS_DIR` |
| `config::routines` | `ROUTINES_ENABLED`, `ROUTINES_MAX_CONCURRENT`, `ROUTINES_CRON_INTERVAL` |
| `config::embeddings` | `EMBEDDING_ENABLED`, `EMBEDDING_PROVIDER`, `EMBEDDING_MODEL`, `EMBEDDING_DIMENSION`, `OPENAI_API_KEY` |
| `config::tunnel` | `TUNNEL_URL`, `TUNNEL_PROVIDER`, `TUNNEL_CF_TOKEN`, `TUNNEL_NGROK_TOKEN`, `TUNNEL_TS_FUNNEL`, `TUNNEL_TS_HOSTNAME`, `TUNNEL_CUSTOM_COMMAND` |

**LLM API key injection via secrets:**

LLM API keys entered during onboarding are stored encrypted in the secrets store (not as plain environment variables). During `init_secrets()`, `inject_llm_keys_from_secrets()` reads these encrypted values and places them in a thread-safe `OnceLock<HashMap<String, String>>` called `INJECTED_VARS`. The `optional_env()` helper function checks real environment variables first, then falls back to this overlay, so explicitly set environment variables always take precedence.

---

## 9. Security Model

IronClaw implements a defense-in-depth security model. Each layer provides independent protection; compromising one layer does not automatically compromise the others.

```
[User Input]
     │
     ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  LAYER 1: Hook Gateway (BeforeInbound)                                  │
│  HookRegistry can reject, modify, or log inbound messages               │
│  before they reach any processing logic.                                │
└──────────────────────────────────────┬──────────────────────────────────┘
                                       │
                                       ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  LAYER 2: Safety Layer (prompt injection defense)                       │
│                                                                         │
│  ┌─────────────┐  ┌─────────────┐  ┌──────────────┐  ┌─────────────┐  │
│  │  Sanitizer  │  │  Validator  │  │    Policy    │  │LeakDetector │  │
│  │  (pattern   │  │  (length,   │  │ (Block/Warn/ │  │ (15+ secret │  │
│  │   detection,│  │  encoding,  │  │  Sanitize/   │  │  patterns,  │  │
│  │   escaping) │  │  forbidden) │  │  Review)     │  │  Block/Redact│  │
│  └─────────────┘  └─────────────┘  └──────────────┘  └─────────────┘  │
│                                                                         │
│  All tool output is wrapped: <tool_output name="..." sanitized="true"> │
│  XML tags create structural boundary between trusted/untrusted content  │
└──────────────────────────────────────┬──────────────────────────────────┘
                                       │
                                       ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  LAYER 3: WASM Sandbox (for untrusted tool code)                        │
│                                                                         │
│  - Capability-based permissions: tools declare allowed endpoints        │
│  - Fuel metering: compute time limits enforced by wasmtime              │
│  - Memory limits: linear memory bounded per module                      │
│  - No ambient authority: WASM can only call declared host functions     │
│  - Per-tool rate limiting prevents denial-of-service via rapid calls    │
│  - Tool shadowing protection: built-in names cannot be overwritten      │
└──────────────────────────────────────┬──────────────────────────────────┘
                                       │ HTTP calls from WASM
                                       ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  LAYER 4: SSRF/Network Proxy (HTTP allowlist enforcement)               │
│                                                                         │
│  DomainAllowlist: only explicitly listed domains are reachable          │
│  CONNECT tunnel validation: HTTPS target domain checked before tunnel   │
│  NetworkPolicyDecider trait: custom allow/deny/inject per request       │
│  CredentialResolver: injects auth headers at transit time               │
│                                                                         │
│  Secrets never enter container/WASM memory — injected by host proxy     │
└──────────────────────────────────────┬──────────────────────────────────┘
                                       │
                                       ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  LAYER 5: Secrets Store (zero-exposure credential model)                │
│                                                                         │
│  - AES-256-GCM encryption at rest                                       │
│  - HKDF-SHA256 per-secret key derivation (different key per secret)     │
│  - Master key from OS keychain (macOS: Keychain, Linux: GNOME/KWallet)  │
│  - SECRETS_MASTER_KEY env var for CI/container deployments              │
│  - LeakDetector scans HTTP responses before returning to WASM           │
│  - Credentials in ~/.ironclaw/.env protected with chmod 600             │
└──────────────────────────────────────┬──────────────────────────────────┘
                                       │
                                       ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  LAYER 6: Docker Isolation (container-domain tools)                     │
│                                                                         │
│  - Per-job Docker containers (bollard crate)                            │
│  - Per-job bearer tokens (not shared across jobs)                       │
│  - Filesystem isolation: workspace mount with policy-controlled access  │
│  - SandboxPolicy: ReadOnly / WorkspaceWrite / FullAccess                │
│  - Container processes never have raw credential values                 │
│  - Network traffic routed through host-side proxy (LAYER 4)             │
└─────────────────────────────────────────────────────────────────────────┘
```

**Skill trust model:**

Skills loaded from user directories (`~/.ironclaw/skills/`, workspace `skills/`) receive `SkillTrust::Trusted` and have access to all tools. Skills downloaded from ClawHub receive `SkillTrust::Installed` and are restricted to read-only tools (no shell, no file writes, no HTTP). When multiple skills are active, `attenuate_tools()` applies the lowest-trust ceiling: a single `Installed` skill reduces the entire turn's tool access to read-only.

**Authentication for the web gateway:**

Bearer token authentication uses a timing-safe comparison via the `subtle` crate's `ConstantTimeEq` trait. This prevents timing side-channel attacks that could reveal the token length or prefix through response time measurement.

**Shell environment scrubbing:**

The shell tool scrubs sensitive environment variable prefixes from the subprocess environment before execution, preventing `env` or `printenv` commands from leaking API keys present in the parent process environment.

---

## 10. Storage and Persistence

IronClaw supports two database backends, selected at compile time via Cargo feature flags and at runtime via the `DATABASE_BACKEND` environment variable.

**Backend comparison:**

| Aspect | PostgreSQL (`postgres` feature) | libSQL/Turso (`libsql` feature) |
|--------|--------------------------------|--------------------------------|
| Use case | Production, existing deployments, multi-user | Zero-dependency local, edge, Turso cloud |
| Embeddings | pgvector extension (native `VECTOR(1536)`) | `F32_BLOB(1536)` with `libsql_vector_idx` |
| Full-text search | `tsvector` / `ts_rank_cd` with GIN index | FTS5 virtual table with sync triggers |
| Schema management | `refinery` versioned migrations | Consolidated `CREATE TABLE IF NOT EXISTS` in `libsql_migrations.rs` |
| Connection model | `deadpool-postgres` connection pool | Per-operation connection (no pool) |
| JSON | `JSONB` with `jsonb_set` path updates | `TEXT` with RFC 7396 JSON Merge Patch |
| UUID | Native `UUID` type | `TEXT` (RFC 4122 string) |
| Timestamps | `TIMESTAMPTZ` | `TEXT` (ISO-8601) |
| Encryption at rest | PostgreSQL TDE / OS-level | No native encryption (use full-disk encryption) |

**Database trait:**

`src/db/mod.rs` defines the `Database` trait with approximately sixty async methods covering all persistence needs:

- Conversations and messages
- Agent jobs, job actions, LLM calls, estimation snapshots
- Sandbox jobs and job events
- Routines and routine run history
- Tool failures (for self-repair tracking)
- Settings (per-user key-value store)
- Workspace: memory documents, memory chunks, hybrid search, heartbeat state
- WASM tools and tool capabilities
- Secrets (encrypted credential storage)

**Workspace storage:**

The workspace uses a virtual filesystem abstraction backed by two database tables:

- `memory_documents` — one row per file (path, content, timestamps, user/agent scope)
- `memory_chunks` — content split into 800-token chunks with 15% overlap, each chunk having an FTS index entry and an optional 1536-dimensional embedding vector

Hybrid search combines BM25 full-text results and cosine-similarity vector results using Reciprocal Rank Fusion (RRF), with configurable weights. When embeddings are not configured, only FTS is used.

**Embedding providers:**

- `OpenAiEmbeddings` — `text-embedding-3-small` (1536 dims) or `text-embedding-3-large` (3072 dims)
- `NearAiEmbeddings` — configurable model via NEAR AI proxy
- `OllamaEmbeddings` — local model inference
- `MockEmbeddings` — deterministic test implementation

**Identity files and system prompt:**

On first boot, `Workspace::seed_if_empty()` creates six core files: `README.md`, `MEMORY.md`, `IDENTITY.md`, `SOUL.md`, `AGENTS.md`, `USER.md`, and `HEARTBEAT.md`. The user edits these to shape the agent's behavior. At each turn, `Workspace::system_prompt()` loads and concatenates these files (plus the last two days of daily logs) to form the LLM system prompt.

**Secrets storage:**

All secrets are encrypted before storage. The encryption scheme:

1. Master key from OS keychain or `SECRETS_MASTER_KEY` env var (32 random bytes)
2. Per-secret key = HKDF-SHA256(master_key, salt=secret_name, info=user_id)
3. Ciphertext = AES-256-GCM(per_secret_key, plaintext, random_nonce)
4. Stored: Base64(nonce || ciphertext) in the `secrets` table

This means compromising one secret's storage does not reveal keys for other secrets, and the master key is never stored in the database.

**Migration system:**

PostgreSQL migrations use `refinery` with versioned SQL files in `migrations/`. The initial schema (`V1__initial.sql`) is 351 lines and includes pgvector indexes, FTS indexes, PL/pgSQL functions, and seed data for leak detection patterns.

The libSQL backend uses a consolidated schema in `src/db/libsql_migrations.rs` (~480 lines) using `CREATE TABLE IF NOT EXISTS` statements. Schema updates require manual `ALTER TABLE` workarounds since incremental migration versioning is not yet implemented for this backend.

---

## 11. Key Design Patterns

**Shared mutable state with `Arc<Mutex<T>>` and `Arc<RwLock<T>>`**

All state shared across tokio tasks uses `Arc` for reference counting and `Mutex` or `RwLock` for interior mutability. The choice follows a consistent rule: use `RwLock` when reads are more frequent than writes (e.g., `ToolRegistry` uses `RwLock<HashMap<...>>` since tools are registered once but looked up on every tool call). Use `Mutex` for session state where a single exclusive lock is simpler and contention is low (e.g., `Arc<Mutex<Session>>`).

```rust
// ToolRegistry: many concurrent readers, rare writes
pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
    builtin_names: RwLock<std::collections::HashSet<String>>,
}

// Session state: exclusive lock, short-lived critical sections
pub struct SessionManager {
    sessions: RwLock<HashMap<String, Arc<Mutex<Session>>>>,
}
```

**mpsc channels for loose coupling between components**

Tokio `mpsc` channels decouple the agent loop from background subsystems. The `ChannelManager` exposes an `inject_sender()` that background tasks (job monitors, routine engine, heartbeat) use to push synthetic `IncomingMessage` values into the main stream. This means background notifications flow through the same message handling path as user input, with no special-casing in the event loop.

```rust
// ChannelManager internal structure
pub struct ChannelManager {
    channels: Arc<RwLock<HashMap<String, Box<dyn Channel>>>>,
    inject_tx: mpsc::Sender<IncomingMessage>,
    inject_rx: tokio::sync::Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
}
```

**Trait objects (`Box<dyn T>` and `Arc<dyn T>`) for polymorphism**

Every major extension point is expressed as a trait. This allows swapping implementations without changing callsites:

- `Arc<dyn LlmProvider>` — primary and failover LLM providers, all wrapping strategies
- `Arc<dyn Tool>` — built-in, WASM, and MCP tools in the same registry
- `Arc<dyn Database>` — PostgreSQL and libSQL backends
- `Arc<dyn SecretsStore + Send + Sync>` — PostgreSQL and libSQL secrets stores
- `Arc<dyn EmbeddingProvider>` — OpenAI, NEAR AI, and Ollama embeddings
- `Box<dyn Channel>` — REPL, HTTP, web gateway, WASM channels
- `Arc<dyn NetworkPolicyDecider>` — custom network access policies for sandbox containers
- `Arc<dyn CredentialResolver>` — custom credential resolution for the network proxy

**`secrecy::Secret<T>` for sensitive values**

All API keys, tokens, and credentials are wrapped in `secrecy::SecretString` or `secrecy::SecretBox<T>`. These types implement `Debug` with redacted output (`[REDACTED]`), preventing accidental logging of secrets via `{:?}` format. Accessing the inner value requires explicitly calling `.expose_secret()`, making all credential exposures visible in code review.

```rust
// Config field for NEAR AI API key
pub api_key: Option<secrecy::SecretString>,

// Usage — explicit expose required
client.api_key(oai.api_key.expose_secret())
```

**`thiserror` + `anyhow` error handling strategy**

Library code (modules under `src/`) defines typed errors using `thiserror`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("Invalid parameters: {0}")]
    InvalidParameters(String),
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Timeout after {0:?}")]
    Timeout(Duration),
}
```

Application-level code (`main.rs`, `app.rs`, init phases) uses `anyhow::Error` for flexible error composition and context chaining via `.context()`. This separates structured errors (where the caller needs to distinguish error types) from opaque errors (where only the message matters).

**Protected built-in tool names**

`ToolRegistry` maintains a `HashSet<String>` of built-in tool names registered via `register_sync()`. When a dynamic registration arrives via `register()` (the async path used by WASM loaders and MCP clients), the registry checks this set first. A dynamically registered tool cannot shadow `shell`, `memory_write`, `create_job`, or any other protected name. This prevents a malicious WASM tool from replacing a security-critical built-in with an adversarial implementation.

**Builder pattern for complex initialization**

`AppBuilder` encapsulates the five-phase initialization sequence and the accumulated state between phases (the database handle is needed to construct the secrets store; the secrets store is needed to inject LLM keys before constructing the LLM provider). Each phase is an async method that updates `self` and returns `Result<(), anyhow::Error>`. The final `build_all()` method runs all phases and returns `AppComponents`.

**State machine for job lifecycle**

Job state transitions are encoded in `src/context/state.rs`:

```
Pending
  └──▶ InProgress
           ├──▶ Completed ──▶ Submitted ──▶ Accepted
           ├──▶ Failed
           └──▶ Stuck ──▶ InProgress  (self-repair recovery)
                     └──▶ Failed      (max repair attempts exceeded)
```

Only valid transitions are permitted. The `SelfRepair` module polls for jobs in the `Stuck` state at a configurable interval and attempts recovery by retrying the last action or marking the job as permanently failed.

---

## 12. Cross-Module Statistics

File counts for each module directory (`.rs` files only, excluding tests in separate files):

| Module | Directory | `.rs` Files |
|--------|-----------|------------|
| `agent` | `src/agent/` | 21 |
| `channels` | `src/channels/` | 35+ |
| `cli` | `src/cli/` | 11 |
| `config` | `src/config/` | 17 |
| `context` | `src/context/` | 4 |
| `db` | `src/db/` | 11 |
| `estimation` | `src/estimation/` | 5 |
| `evaluation` | `src/evaluation/` | 3 |
| `extensions` | `src/extensions/` | 4 |
| `history` | `src/history/` | 3 |
| `hooks` | `src/hooks/` | 5 |
| `llm` | `src/llm/` | 12 |
| `observability` | `src/observability/` | 5 |
| `orchestrator` | `src/orchestrator/` | 4 |
| `pairing` | `src/pairing/` | 2 |
| `registry` | `src/registry/` | 4 |
| `safety` | `src/safety/` | 5 |
| `sandbox` | `src/sandbox/` | 9 |
| `secrets` | `src/secrets/` | 5 |
| `setup` | `src/setup/` | 4 |
| `skills` | `src/skills/` | 7 |
| `tools` | `src/tools/` | 45+ |
| `tunnel` | `src/tunnel/` | 6 |
| `worker` | `src/worker/` | 5 |
| `workspace` | `src/workspace/` | 7 |
| **Top-level files** | `src/*.rs` | 11 (`main.rs`, `lib.rs`, `app.rs`, `bootstrap.rs`, `service.rs`, `error.rs`, `settings.rs`, `util.rs`, `boot_screen.rs`, `testing.rs`, `tracing_fmt.rs`) |

> **Note**: File counts updated for v0.9.0. The tools module now includes 12 files in `builtin/`, 13 files in `wasm/`, and additional builder/mcp files.

The `tools` module is one of the largest modules, reflecting the breadth of the tool system: built-ins, a full WASM runtime, an MCP client, a software builder, and the registry/trait definitions. The `channels` module includes REPL, web gateway, HTTP, and WASM channel runtime implementations.

**Key third-party crate dependencies:**

| Crate | Version | Role |
|-------|---------|------|
| `tokio` | 1.x | Async runtime |
| `axum` | 0.8 | HTTP server (web gateway, webhook, orchestrator API) |
| `rig-core` | 0.30 | Multi-provider LLM abstraction (OpenAI, Anthropic, Ollama) |
| `wasmtime` | 28 | WASM execution engine with component model support |
| `bollard` | 0.18 | Docker API client for container lifecycle management |
| `refinery` | 0.8 | PostgreSQL schema migration runner |
| `libsql` | 0.6 | Embedded SQLite/Turso database backend |
| `deadpool-postgres` | 0.14 | PostgreSQL connection pool |
| `secrecy` | 0.10 | Redacted wrapper types for sensitive values |
| `aes-gcm` | 0.10 | AES-256-GCM authenticated encryption |
| `hkdf` | 0.12 | HKDF key derivation for per-secret keys |
| `thiserror` | 2.x | Typed error derivation for library code |
| `anyhow` | 1.x | Flexible error handling for application code |
| `tracing` | 0.1 | Structured logging and diagnostics |
| `clap` | 4.x | CLI argument parsing with derive macros |
| `rustyline` | 17.x | REPL line editing, history, completion |
| `termimad` | 0.34 | Markdown rendering in terminal REPL |
| `pgvector` | 0.4 | PostgreSQL vector type support for semantic search |
| `regex` | 1.x | Pattern matching for safety layer and skill scoring |
| `serde_yaml` | 0.9.x | YAML parsing for SKILL.md frontmatter |
| `dotenvy` | 0.15 | `.env` file loading |
| `hyper` | 1.5 | HTTP/1.1 and HTTP/2 server for network proxy |
| `subtle` | 2.x | Constant-time comparison for auth token validation |

---

*Document generated from source code inspection of IronClaw v0.9.0 (`src/` directory). For module-level specifications, see `src/setup/README.md`, `src/workspace/README.md`, and `src/tools/README.md`.*
