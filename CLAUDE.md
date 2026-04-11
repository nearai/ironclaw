# IronClaw Development Guide

**IronClaw** is a secure personal AI assistant — user-first security, self-expanding tools, defense in depth, multi-channel access with proactive background execution.

## Build & Test

```bash
cargo fmt                                                    # format
cargo clippy --all --benches --tests --examples --all-features  # lint (zero warnings)
cargo test                                                   # unit tests
cargo test --features integration                            # + PostgreSQL tests
RUST_LOG=ironclaw=debug cargo run                            # run with logging
```

E2E tests: see `tests/e2e/CLAUDE.md`.

## Code Style

- Prefer `crate::` for cross-module imports; `super::` is fine in tests and intra-module refs
- No `pub use` re-exports unless exposing to downstream consumers
- No `.unwrap()` or `.expect()` in production code (tests are fine)
- Use `thiserror` for error types in `error.rs`
- Map errors with context: `.map_err(|e| SomeError::Variant { reason: e.to_string() })?`
- Prefer strong types over strings (enums, newtypes)
- Keep functions focused, extract helpers when logic is reused
- Comments for non-obvious logic only
- **Prompt templates live in files, not Rust code**: Multi-line prompt strings (mission goals, system prompts, CodeAct preambles) go in `crates/ironclaw_engine/prompts/*.md` and are loaded via `include_str!()`. Never inline large prompt templates as Rust string constants — they're hard to read, review, and iterate on. Single-line format strings are fine inline.
- **Logging levels matter for REPL/TUI**: `info!` and `warn!` output appears in the REPL and corrupts the terminal UI. Use `debug!` for internal diagnostics (trace analysis, reflection results, engine internals). Reserve `info!` for user-facing status that the REPL intentionally renders. Background tasks (reflection, trace analysis) must NEVER use `info!` — it breaks the interactive display.
- **Test through the caller, not just the helper**: When a predicate/classifier/transform helper gates a side effect (HTTP, DB write, OAuth, UI mutation, tool execution) and has any wrapper or computed input between it and that side effect, a unit test on the helper alone is *not* sufficient regression coverage. Add a test that drives the call site — typically a `*_handler`, `factory::create_*`, or `manager::*` — at the integration tier (`cargo test --features integration`) or higher. The same applies to test mocks: if you mock a multi-arg runtime API like `window.open(url, target, features)`, the mock must capture every argument the production caller passes. See `.claude/rules/testing.md` ("Test Through the Caller, Not Just the Helper") for the full rule and the bug examples that motivated it.

## Architecture

Prefer generic/extensible architectures over hardcoding specific integrations. Ask clarifying questions about the desired abstraction level before implementing.

Key traits for extensibility: `Database`, `Channel`, `Tool`, `LlmProvider`, `SuccessEvaluator`, `EmbeddingProvider`, `NetworkPolicyDecider`, `Hook`, `Observer`, `Tunnel`.

All I/O is async with tokio. Use `Arc<T>` for shared state, `RwLock` for concurrent access.

**LLM data is never deleted.** All LLM output — context fed to the model, reasoning, tool calls, messages, events, steps — is the most valuable data in the system. Never strip, truncate, or delete it from the database. Mark with timestamps, make filterable, but always retain. In-memory HashMaps are caches; the database (via Workspace) is the source of truth. "Cleanup" means evicting from in-memory caches, never deleting database rows.

## Extracted Crates

Safety logic lives in `crates/ironclaw_safety/`, skills in `crates/ironclaw_skills/`. **Import directly from the extracted crate** (e.g. `use ironclaw_safety::SafetyLayer`, `use ironclaw_skills::SkillRegistry`). Do not use `crate::safety::` or `crate::skills::` for types that originate in extracted crates — `src/safety/mod.rs` and `src/skills/mod.rs` no longer glob-re-export. Local items defined in those modules (e.g. `crate::skills::attenuate_tools`) are fine.

## Project Structure

```
crates/
└── ironclaw_safety/    # Extracted: prompt injection, validation, leak detection, policy

src/
├── lib.rs              # Library root, module declarations
├── main.rs             # Entry point, CLI args, startup
├── app.rs              # App startup orchestration (channel wiring, DB init)
├── bootstrap.rs        # Base directory resolution (~/.ironclaw), early .env loading
├── settings.rs         # User settings persistence (~/.ironclaw/settings.json)
├── service.rs          # OS service management (launchd/systemd daemon install)
├── tracing_fmt.rs      # Custom tracing formatter
├── util.rs             # Shared utilities
├── config/             # Configuration from env vars (split by subsystem)
│   ├── mod.rs          # Re-exports all config types; top-level Config struct
│   ├── agent.rs, llm.rs, channels.rs, database.rs, sandbox.rs, skills.rs
│   ├── heartbeat.rs, routines.rs, safety.rs, embeddings.rs, wasm.rs
│   ├── tunnel.rs       # Tunnel provider config (TUNNEL_PROVIDER, TUNNEL_URL, etc.)
│   └── secrets.rs, hygiene.rs, builder.rs, helpers.rs
├── error.rs            # Error types (thiserror)
│
├── agent/              # Core agent loop, dispatcher, scheduler, sessions — see src/agent/CLAUDE.md
│
├── channels/           # Multi-channel input
│   ├── channel.rs      # Channel trait, IncomingMessage, OutgoingResponse
│   ├── manager.rs      # ChannelManager merges streams
│   ├── cli/            # Full TUI with Ratatui
│   ├── http.rs         # HTTP webhook (axum) with secret validation
│   ├── webhook_server.rs # Unified HTTP server composing all webhook routes
│   ├── repl.rs         # Simple REPL (for testing)
│   ├── web/            # Web gateway (browser UI) — see src/channels/web/CLAUDE.md
│   └── wasm/           # WASM channel runtime
│       ├── mod.rs
│       ├── bundled.rs  # Bundled channel discovery
│       ├── capabilities.rs # Channel-specific capabilities (HTTP endpoint, emit rate)
│       ├── error.rs    # WASM channel error types
│       ├── runtime.rs  # WASM channel execution runtime
│       ├── setup.rs    # WasmChannelSetup, setup_wasm_channels(), inject_channel_credentials()
│       └── wrapper.rs  # Channel trait wrapper for WASM modules
│
├── cli/                # CLI subcommands (clap)
│   ├── mod.rs          # Cli struct, Command enum (run/onboard/config/tool/registry/mcp/memory/pairing/service/doctor/status/completion)
│   └── config.rs, tool.rs, registry.rs, mcp.rs, memory.rs, pairing.rs, service.rs, doctor.rs, status.rs, completion.rs
│
├── registry/           # Extension registry catalog
│   ├── manifest.rs     # ExtensionManifest, ArtifactSpec, BundleDefinition types
│   ├── catalog.rs      # RegistryCatalog: load from filesystem and embedded JSON
│   └── installer.rs    # RegistryInstaller: download, verify, install WASM artifacts
│
├── hooks/              # Lifecycle hooks (6 points: BeforeInbound, BeforeToolCall, BeforeOutbound, OnSessionStart, OnSessionEnd, TransformResponse)
│
├── tunnel/             # Tunnel abstraction for public internet exposure
│   ├── mod.rs          # Tunnel trait, TunnelProviderConfig, create_tunnel(), start_managed_tunnel()
│   ├── cloudflare.rs   # CloudflareTunnel (cloudflared binary)
│   ├── ngrok.rs        # NgrokTunnel
│   ├── tailscale.rs    # TailscaleTunnel (serve/funnel modes)
│   ├── custom.rs       # CustomTunnel (arbitrary command with {host}/{port})
│   └── none.rs         # NoneTunnel (local-only, no exposure)
│
├── observability/      # Pluggable event/metric recording (noop, log, multi)
│
├── orchestrator/       # Internal HTTP API for sandbox containers
│   ├── api.rs          # Axum endpoints (LLM proxy, events, prompts)
│   ├── auth.rs         # Per-job bearer token store
│   └── job_manager.rs  # Container lifecycle (create, stop, cleanup)
│
├── worker/             # Runs inside Docker containers
│   ├── container.rs    # Container worker runtime (ContainerDelegate + shared agentic loop)
│   ├── job.rs          # Background job worker (JobDelegate + shared agentic loop)
│   ├── claude_bridge.rs # Claude Code bridge (spawns claude CLI)
│   └── proxy_llm.rs    # LlmProvider that proxies through orchestrator
│
├── safety/             # Re-export shim for crates/ironclaw_safety (see Extracted Crates)
│
├── llm/                # Multi-provider LLM integration — see src/llm/CLAUDE.md
│
├── tools/              # Extensible tool system
│   ├── tool.rs         # Tool trait, ToolOutput, ToolError
│   ├── registry.rs     # ToolRegistry for discovery
│   ├── rate_limiter.rs # Shared sliding-window rate limiter
│   ├── builtin/        # Built-in tools (echo, time, json, http, web_fetch, file, shell, memory, message, job, routine, extension_tools, skill_tools, secrets_tools)
│   ├── builder/        # Dynamic tool building
│   │   ├── core.rs     # BuildRequirement, SoftwareType, Language
│   │   ├── templates.rs # Project scaffolding
│   │   ├── testing.rs  # Test harness integration
│   │   └── validation.rs # WASM validation
│   ├── mcp/            # Model Context Protocol
│   │   ├── client.rs   # MCP client over HTTP
│   │   ├── factory.rs  # create_client_from_config() — transport dispatch factory
│   │   ├── protocol.rs # JSON-RPC types
│   │   └── session.rs  # MCP session management (Mcp-Session-Id header, per-server state)
│   └── wasm/           # Full WASM sandbox (wasmtime)
│       ├── runtime.rs  # Module compilation and caching
│       ├── wrapper.rs  # Tool trait wrapper for WASM modules
│       ├── host.rs     # Host functions (logging, time, workspace)
│       ├── limits.rs   # Fuel metering and memory limiting
│       ├── allowlist.rs # Network endpoint allowlisting
│       ├── credential_injector.rs # Safe credential injection
│       ├── loader.rs   # WASM tool discovery from filesystem
│       ├── rate_limiter.rs # Per-tool rate limiting
│       ├── error.rs    # WASM-specific error types
│       └── storage.rs  # Linear memory persistence
│
├── db/                 # Dual-backend persistence (PostgreSQL + libSQL) — see src/db/CLAUDE.md
│
├── workspace/          # Persistent memory system — see src/workspace/README.md
│
├── context/            # Job context isolation (JobState, JobContext, ContextManager)
├── estimation/         # Cost/time/value estimation with EMA learning
├── evaluation/         # Success evaluation (rule-based, LLM-based)
│
├── sandbox/            # Docker execution sandbox
│   ├── config.rs       # SandboxConfig, SandboxPolicy enum (ReadOnly/WorkspaceWrite/FullAccess)
│   ├── manager.rs      # SandboxManager orchestration
│   ├── container.rs    # ContainerRunner, Docker lifecycle
│   └── proxy/          # Network proxy: domain allowlist, credential injection, CONNECT tunnel
│
├── secrets/            # Secrets management (AES-256-GCM, OS keychain for master key)
│
├── profile.rs          # Psychographic profile types, 9-dimension analysis framework
│
├── setup/              # 7-step onboarding wizard — see src/setup/README.md
│
├── skills/             # SKILL.md prompt extension system — see .claude/rules/skills.md
│
└── history/            # Persistence (PostgreSQL repositories, analytics)

tests/
├── *.rs                # Integration tests (workspace, heartbeat, WS gateway, pairing, etc.)
├── test-pages/         # HTML→Markdown conversion fixtures
└── e2e/                # Python/Playwright E2E scenarios (see tests/e2e/CLAUDE.md)
```

## Database

Dual-backend: PostgreSQL + libSQL/Turso. **All new persistence features must support both backends.** See `src/db/CLAUDE.md` and `.claude/rules/database.md`.

## Module Specs

When modifying a module with a spec, read the spec first. Code follows spec; spec is the tiebreaker.

**Module-owned initialization:** Module-specific initialization logic (database connection, transport creation, channel setup) must live in the owning module as a public factory function — not in `main.rs` or `app.rs`. These entry-point files orchestrate calls to module factories. Feature-flag branching (`#[cfg(feature = ...)]`) must be confined to the module that owns the abstraction.

| Module | Spec |
|--------|------|
| `src/agent/` | `src/agent/CLAUDE.md` |
| `src/channels/web/` | `src/channels/web/CLAUDE.md` |
| `src/db/` | `src/db/CLAUDE.md` |
| `src/llm/` | `src/llm/CLAUDE.md` |
| `src/setup/` | `src/setup/README.md` |
| `src/tools/` | `src/tools/README.md` |
| `src/workspace/` | `src/workspace/README.md` |
| `crates/ironclaw_engine/` | `crates/ironclaw_engine/CLAUDE.md` |
| `tests/e2e/` | `tests/e2e/CLAUDE.md` |

## Job State Machine

```
Pending -> InProgress -> Completed -> Submitted -> Accepted
    \                \-> Failed
     \-> Failed       \-> Stuck -> InProgress (recovery)
                              \-> Failed
```

## Skills System

SKILL.md files extend the agent's prompt with domain-specific instructions. See `.claude/rules/skills.md` for full details.

- **Trust model**: Trusted (user-placed in `~/.ironclaw/skills/` or workspace `skills/`, full tool access) vs Installed (registry, read-only tools)
- **Selection pipeline**: gating (check bin/env/config requirements) -> scoring (keywords/patterns/tags) -> budget (fit within `SKILLS_MAX_TOKENS`) -> attenuation (trust-based tool ceiling)
- **Skill tools**: `skill_list`, `skill_search`, `skill_install`, `skill_remove`

## Configuration

See `.env.example` for all environment variables. LLM backends (`nearai`, `openai`, `anthropic`, `ollama`, `openai_compatible`, `tinfoil`, `bedrock`) documented in `src/llm/CLAUDE.md`.

## Adding a New Channel

1. Create `src/channels/my_channel.rs`
2. Implement the `Channel` trait
3. Add config in `src/config/channels.rs`
4. Wire up in `src/app.rs` channel setup section

## Everything Goes Through Tools

**Core principle**: all actions originating from gateway handlers, CLI
commands, routine engine, WASM channels, or any other non-agent caller
MUST go through `ToolDispatcher::dispatch()` — never directly through
`state.store`, `workspace`, `extension_manager`, `skill_registry`, or
`session_manager`.

This gives every UI-initiated mutation the same audit trail
(`ActionRecord`), safety pipeline (param validation, sensitive-param
redaction, output sanitization), and channel-agnostic surface as
agent-initiated tool calls. Channels are interchangeable extensions;
routing through one dispatch function means new channels inherit the
full pipeline for free.

The pre-commit hook (`scripts/pre-commit-safety.sh`) flags newly-added
lines in handler/CLI files that touch
`state.{store,workspace,extension_manager,skill_registry,session_manager}.*`
directly. Annotate intentional exceptions (rare — usually only read
aggregation across multiple users) with a trailing
`// dispatch-exempt: <reason>` comment on the same line. The check only
sees added lines, so existing untouched code doesn't trip during
incremental migration.

See `.claude/rules/tools.md` for the full pattern, allowed exemptions,
and migration status. The dispatcher itself lives in
`src/tools/dispatch.rs`.

## Engine v2 Per-Project Sandbox

When `SANDBOX_ENABLED=true` (or `ENGINE_V2_SANDBOX=true`), engine v2 routes the five filesystem/shell tools
(`file_read`, `file_write`, `list_dir`, `apply_patch`, `shell`) for `/project/`
paths through a per-project Docker container instead of the host filesystem.
The host's directory at `~/.ironclaw/projects/<user_id>/<project_id>/` is bind-mounted at
`/project/` inside the container, and a `sandbox_daemon` binary inside the
container speaks NDJSON over `docker exec -i`.

When unset, the same code path uses a host-filesystem `MountBackend` —
behavior is unchanged. See `docs/plans/2026-04-10-engine-v2-sandbox.md`.

Build the sandbox image: `docker build -f crates/Dockerfile.sandbox -t ironclaw/sandbox:dev .`

## Workspace & Memory

Persistent memory with hybrid search (FTS + vector via RRF). Four tools: `memory_search`, `memory_write`, `memory_read`, `memory_tree`. Identity files (AGENTS.md, SOUL.md, USER.md, IDENTITY.md) injected into system prompt. Heartbeat system runs proactive periodic execution (default: 30 minutes), reading `HEARTBEAT.md` and notifying via channel if findings. See `src/workspace/README.md`.

## Debugging

```bash
RUST_LOG=ironclaw=trace cargo run           # verbose
RUST_LOG=ironclaw::agent=debug cargo run    # agent module only
RUST_LOG=ironclaw=debug,tower_http=debug cargo run  # + HTTP request logging
```

## Current Limitations

1. Domain-specific tools (`marketplace.rs`, `restaurant.rs`, etc.) are stubs
2. Integration tests need testcontainers for PostgreSQL
3. MCP: no streaming support; stdio/HTTP/Unix transports all use request-response
4. WIT bindgen: auto-extract tool schema from WASM is stubbed
5. Built tools get empty capabilities; need UX for granting access
6. No tool versioning or rollback
7. Observability: only `log` and `noop` backends (no OpenTelemetry)
