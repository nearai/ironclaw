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

- Use `crate::` imports, not `super::`
- No `pub use` re-exports unless exposing to downstream consumers
- No `.unwrap()` or `.expect()` in production code (tests are fine)
- Use `thiserror` for error types in `error.rs`
- Map errors with context: `.map_err(|e| SomeError::Variant { reason: e.to_string() })?`
- Prefer strong types over strings (enums, newtypes)
- Keep functions focused, extract helpers when logic is reused
- Comments for non-obvious logic only

## Architecture

Prefer generic/extensible architectures over hardcoding specific integrations. Ask clarifying questions about the desired abstraction level before implementing.

Key traits for extensibility: `Database`, `Channel`, `Tool`, `LlmProvider`, `SuccessEvaluator`, `EmbeddingProvider`, `NetworkPolicyDecider`, `Hook`, `Observer`, `Tunnel`.

All I/O is async with tokio. Use `Arc<T>` for shared state, `RwLock` for concurrent access.

## Project Structure

```
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
├── tunnel/             # Tunnel abstraction (cloudflare, ngrok, tailscale, custom, none)
│
├── observability/      # Pluggable event/metric recording (noop, log, multi)
│
├── orchestrator/       # Internal HTTP API for sandbox containers
│   ├── api.rs          # Axum endpoints (LLM proxy, events, prompts)
│   ├── auth.rs         # Per-job bearer token store
│   └── job_manager.rs  # Container lifecycle (create, stop, cleanup)
│
├── worker/             # Runs inside Docker containers
│   ├── runtime.rs      # Worker execution loop (tool calls, LLM)
│   ├── claude_bridge.rs # Claude Code bridge (spawns claude CLI)
│   └── proxy_llm.rs    # LlmProvider that proxies through orchestrator
│
├── safety/             # Prompt injection defense
│   ├── sanitizer.rs    # Pattern detection, content escaping
│   ├── validator.rs    # Input validation (length, encoding, patterns)
│   ├── policy.rs       # PolicyRule system with severity/actions
│   ├── leak_detector.rs # Secret detection (API keys, tokens, etc.)
│   └── credential_detect.rs # HTTP request credential detection
│
├── llm/                # Multi-provider LLM integration — see src/llm/CLAUDE.md
│
├── tools/              # Extensible tool system
│   ├── tool.rs         # Tool trait, ToolOutput, ToolError
│   ├── registry.rs     # ToolRegistry for discovery
│   ├── rate_limiter.rs # Shared sliding-window rate limiter
│   ├── builtin/        # Built-in tools (echo, time, json, http, web_fetch, file, shell, memory, message, job, routine, extension_tools, skill_tools, secrets_tools)
│   ├── builder/        # Dynamic tool building
│   ├── mcp/            # Model Context Protocol client
│   └── wasm/           # Full WASM sandbox (wasmtime) — runtime, host functions, fuel metering, allowlist, credential injection
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
├── setup/              # 8-step onboarding wizard — see src/setup/README.md
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

| Module | Spec |
|--------|------|
| `src/agent/` | `src/agent/CLAUDE.md` |
| `src/channels/web/` | `src/channels/web/CLAUDE.md` |
| `src/db/` | `src/db/CLAUDE.md` |
| `src/llm/` | `src/llm/CLAUDE.md` |
| `src/setup/` | `src/setup/README.md` |
| `src/tools/` | `src/tools/README.md` |
| `src/workspace/` | `src/workspace/README.md` |
| `tests/e2e/` | `tests/e2e/CLAUDE.md` |

## Job State Machine

```
Pending -> InProgress -> Completed -> Submitted -> Accepted
                     \-> Failed
                     \-> Stuck -> InProgress (recovery)
                              \-> Failed
```

## Skills System

SKILL.md files extend the agent's prompt with domain-specific instructions. See `.claude/rules/skills.md` for full details.

- **Trust model**: Trusted (user-placed in `~/.ironclaw/skills/` or workspace `skills/`, full tool access) vs Installed (registry, read-only tools)
- **Selection pipeline**: gating (check bin/env/config requirements) -> scoring (keywords/patterns/tags) -> budget (fit within `SKILLS_MAX_TOKENS`) -> attenuation (trust-based tool ceiling)
- **Skill tools**: `skill_list`, `skill_search`, `skill_install`, `skill_remove`

## Configuration

See `.env.example` for all environment variables. Key settings:

```bash
DATABASE_BACKEND=postgres               # or "libsql" / "turso"
DATABASE_URL=postgres://user:pass@localhost/ironclaw
LLM_BACKEND=nearai                      # nearai/openai/anthropic/ollama/openai_compatible/tinfoil
GATEWAY_ENABLED=true                    # Web gateway
GATEWAY_PORT=3001
GATEWAY_AUTH_TOKEN=changeme
SANDBOX_ENABLED=true                    # Docker sandbox
SANDBOX_DEFAULT_POLICY=workspace_write  # ReadOnly/WorkspaceWrite/FullAccess
SKILLS_ENABLED=true
SKILLS_MAX_TOKENS=4000
ROUTINES_ENABLED=true
HEARTBEAT_ENABLED=true                  # Proactive periodic execution
HEARTBEAT_INTERVAL_SECS=1800            # 30 minutes
EMBEDDING_PROVIDER=nearai               # or openai
TUNNEL_PROVIDER=none                    # none/cloudflare/tailscale/ngrok/custom
OBSERVABILITY_BACKEND=none              # none or log
```

## Adding a New Channel

1. Create `src/channels/my_channel.rs`
2. Implement the `Channel` trait
3. Add config in `src/config/channels.rs`
4. Wire up in `src/app.rs` channel setup section

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
3. MCP: only HTTP transport (no stdio)
4. WIT bindgen: auto-extract tool schema from WASM is stubbed
5. Built tools get empty capabilities; need UX for granting access
6. No tool versioning or rollback
7. Observability: only `log` and `noop` backends (no OpenTelemetry)
