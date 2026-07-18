# IronClaw Development Guide

**IronClaw** is a secure personal AI assistant — user-first security, self-expanding tools, defense in depth, multi-channel access with proactive background execution.

## Code Discovery — Query the Knowledge Graph First

This repo can be indexed into a **codebase knowledge graph** (the `codebase-memory` MCP server) over `src/` and `crates/`. For any *where-is / who-calls / how-does-data-flow / what-does-this-touch* question, **probe the graph before reaching for `Grep`** — text search cannot see cross-crate call chains, and this codebase's real cost is cross-crate (a feature crosses `product_workflow → composition → webui_v2 → runtime → frontend`).

**Where it lives:** `.codebase-memory/graph.db.zst` — a **git-ignored build artifact, not source**. One per environment, rebuilt from code. Never commit it.

**Freshness (check at the start of a discovery task):** run `bash scripts/codebase-graph.sh status` — it compares the graph's indexed commit against `HEAD`. Then:
- **Missing** → `index_repository(repo_path=".")` once to build it.
- **Stale** → `detect_changes(since="<indexed-commit>")` for the changed symbols + blast radius, or re-run `index_repository` to fully refresh.
- The graph is a point-in-time index — verify anything it asserts against live code before acting.

**Discovery recipes (use these instead of `Grep` for code structure):**
- Where a symbol is defined → `search_graph(name_pattern=…)`, then `get_code_snippet(qualified_name=…)`
- Who calls X / what X calls → `trace_path(function_name=…, mode="calls")`
- How a value flows across layers → `trace_path(mode="data_flow")`
- Cross-crate / cross-service path (the reborn 5-layer feature flow) → `trace_path(mode="cross_service")`
- Structure of an area → `get_architecture(…)`; graph-augmented text search → `search_code(pattern=…)`
- Arbitrary structural queries → `query_graph(<Cypher>)`

`Grep`/`Glob`/`Read` remain correct for text, config, and non-code files — and for reading a file the graph pointed you to. For *code structure*, the graph comes first.

**Narrative orientation (what/why, not where):** prose docs for each subsystem live in `openwiki/` — an auto-generated wiki kept fresh by `.github/workflows/openwiki-update.yml`. For *"what does this subsystem do / how does this flow work"* questions, `Read` the relevant `openwiki/` page; use the graph for precise structure. Do not hand-edit `openwiki/` — it is regenerated. The two layers are complementary: `openwiki/` = prose map, the graph = exact index.

## Where to Build — Reborn-First

**New feature work targets the Reborn stack in `crates/`, not the v1 `src/` monolith.** A Reborn feature crosses `product_workflow → composition → webui_v2 → runtime/serve → frontend`; the binary entry point is `crates/ironclaw_reborn_cli` (binary name `ironclaw`), **not** `src/main.rs`. Start from the `reborn-feature` skill — it maps those layers so you wire a feature in one pass instead of layer-by-layer.

`src/` is the **v1 monolith**, being retired under the roadmap's "Clean up old architecture." Maintain existing v1 behavior there when a bug requires it, but **do not build new features into `src/`** — they belong Reborn-side. The detailed `src/` layout in "Project Structure" below documents v1 for maintenance, not as the default place to add code.

## Build & Test

```bash
cargo fmt                                                    # format
cargo clippy --all --benches --tests --examples --all-features  # lint (zero warnings)
cargo test                                                   # unit tests
cargo test --features integration                            # + PostgreSQL tests
RUST_LOG=ironclaw=debug cargo run                            # run with logging
```

E2E tests: see `tests/e2e/CLAUDE.md`.

## Testing Discipline

Two rules are non-negotiable for **all** tests:

1. **Test-first.** Every new feature and every bug fix starts in the
   tests — write or update the test that pins the behavior, watch it
   fail for the right reason, *then* change the implementation. Red,
   then green. (The commit-msg hook already requires a regression test
   with every fix; this is the ordering.)
2. **Consolidate, don't proliferate.** Extensive coverage of every code
   path, with minimal overlap. If a test already exercises most of the
   path, **extend it** (a case, an assertion, a scripted turn) — do not
   stand up a redundant new "extensive" test that overloads the suite.
   Add a new test only for a genuinely distinct scenario, and say why an
   existing one couldn't absorb it.
3. **Integration-first coverage.** Production-wired Reborn behavior
   ships with a test in `tests/integration/`, driven through the
   harness and asserting at a seam — never `wait_for_status(Completed)`
   alone. Crate-tier is the fallback only when that tier can't reach
   the path (say why in the PR). Full decision rule:
   `.claude/rules/testing.md`.

Where to look: hard rules (tiers, test-through-the-caller,
regression-with-every-fix) in `.claude/rules/testing.md`; **Reborn
integration tests** authoring guide in `tests/integration/CLAUDE.md`;
Python/Playwright suite in `tests/e2e/CLAUDE.md`.

## Code Style

- Prefer `crate::` for cross-module imports; `super::` is fine in tests and intra-module refs
- No `pub use` re-exports unless exposing to downstream consumers
- No `.unwrap()` or `.expect()` in production code (tests are fine)
- Use `thiserror` for error types in `error.rs`
- Map errors with context: `.map_err(|e| SomeError::Variant { reason: e.to_string() })?`
- Prefer strong types over strings (enums, newtypes)
- Keep functions focused, extract helpers when logic is reused
- Comments for non-obvious logic only
- **Prompt templates live in files, not Rust code**: Multi-line prompt strings (mission goals, system prompts, preambles) go in a `prompts/*.md` file **inside the crate that owns the behavior** and are loaded via `include_str!()`. Reborn examples: `crates/ironclaw_loop_host`, `crates/ironclaw_turns`, `crates/ironclaw_skill_learning`. Never inline large prompt templates as Rust string constants — they're hard to read, review, and iterate on. Single-line format strings are fine inline.
- **Logging levels matter for REPL/TUI**: `info!` and `warn!` output appears in the REPL and corrupts the terminal UI. Use `debug!` for internal diagnostics (trace analysis, reflection results, engine internals). Reserve `info!` for user-facing status that the REPL intentionally renders. Background tasks (reflection, trace analysis) must NEVER use `info!` — it breaks the interactive display.
- **Test through the caller, not just the helper**: When a predicate/classifier/transform helper gates a side effect (HTTP, DB write, OAuth, UI mutation, tool execution) and has any wrapper or computed input between it and that side effect, a unit test on the helper alone is *not* sufficient regression coverage. Add a test that drives the call site — typically a `*_handler`, `factory::create_*`, or `manager::*` — at the integration tier (`cargo test --features integration`) or higher. The same applies to test mocks: if you mock a multi-arg runtime API like `window.open(url, target, features)`, the mock must capture every argument the production caller passes. See `.claude/rules/testing.md` ("Test Through the Caller, Not Just the Helper") for the full rule and the bug examples that motivated it.

## Architecture

Prefer generic/extensible architectures over hardcoding specific integrations. Ask clarifying questions about the desired abstraction level before implementing.

### Extension/Auth Invariants

Extension and channel onboarding has two distinct identities that must not be conflated:

- `credential_name`: backend secret identity used for storage, injection, and gate resume
- `extension_name`: user-facing installed extension/channel identity used for setup routing and UI

Examples:

- Telegram:
  - `credential_name = telegram_bot_token`
  - `extension_name = telegram`
- Gmail:
  - `credential_name = google_oauth_token`
  - `extension_name = gmail`

Rules:

- Never route web setup/configure UI directly from `credential_name`.
- Chat and Settings must use the same setup/configure path for installable extensions/channels.
- Generic auth-card UI is only for non-extension credential prompts or pure OAuth launch prompts.
- If an auth flow is for an installed extension/channel, resolve the `extension_name` once in shared backend logic and carry it through the wire contract rather than re-deriving it in multiple layers.
- New auth/onboarding code must reuse the shared resolver/controller path instead of adding channel-specific or frontend-only fallbacks.

Current ownership:

- `src/auth/extension.rs`: canonical auth-flow extension-name resolver (`resolve_auth_flow_extension_name` free fn + `AuthManager::resolve_extension_name_for_auth_flow`)
- `src/channels/web/features/chat/mod.rs`: web auth submit routing and history rehydration (`pending_gate_extension_name` wrapper, `pending_auth` handling)
- `crates/ironclaw_gateway/static/js/core/onboarding.js`: unified onboarding controller and configure-modal routing (previously in the monolithic `app.js`, now split — see `crates/ironclaw_gateway/src/assets.rs` for the concat order)

Auth mode:

- Web auth prompts use the legacy `pending_auth` path (`/api/chat/auth-token`, `/api/chat/auth-cancel`); the user's next message is intercepted and routed to the credential store.
- The former engine-v2 gate path (`/api/chat/gate/resolve`, `request_id`-scoped pending gates) has been removed along with engine v2.

Key traits for extensibility: `Database`, `Channel`, `Tool`, `LlmProvider`, `SuccessEvaluator`, `EmbeddingProvider`, `NetworkPolicyDecider`, `Hook`, `Observer`, `Tunnel`.

All I/O is async with tokio. Use `Arc<T>` for shared state, `RwLock` for concurrent access.

**LLM data is never deleted.** All LLM output — context fed to the model, reasoning, tool calls, messages, events, steps — is the most valuable data in the system. Never strip, truncate, or delete it from the database. Mark with timestamps, make filterable, but always retain. In-memory HashMaps are caches; the database (via Workspace) is the source of truth. "Cleanup" means evicting from in-memory caches, never deleting database rows.

## Extracted Crates

Safety logic lives in `crates/ironclaw_safety/`, skills in `crates/ironclaw_skills/`, multi-provider LLM integration in `crates/ironclaw_llm/`. **Import directly from the extracted crate** (e.g. `use ironclaw_safety::SafetyLayer`, `use ironclaw_skills::SkillRegistry`, `use ironclaw_llm::{LlmProvider, LlmError}`). Do not use `crate::safety::`, `crate::skills::`, or `crate::llm::` for types that originate in extracted crates — `src/llm/` was deleted in the LLM extraction, and `src/safety/mod.rs` / `src/skills/mod.rs` no longer glob-re-export. Local items defined in those modules (e.g. `crate::skills::attenuate_tools`) are fine. The `crate::error::LlmError` alias and `crate::config::*Config` re-exports are kept as a thin convenience: they forward to `ironclaw_llm::*` so existing call sites compile, but new code should import from the extracted crate.

## Project Structure

```
crates/
├── ironclaw_safety/    # Extracted: prompt injection, validation, leak detection, policy
└── ironclaw_llm/       # Extracted: multi-provider LLM integration (rig-core, OpenAI, Anthropic, NEAR AI, Bedrock, …)

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
├── safety/             # Docs-only pointer module (no re-exports; import from ironclaw_safety directly)
│
├── (llm/  was extracted to crates/ironclaw_llm/ — see Extracted Crates)
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
| `crates/ironclaw_llm/` | `crates/ironclaw_llm/CLAUDE.md` |
| `crates/ironclaw_embeddings/` | `crates/ironclaw_embeddings/AGENTS.md` |
| `src/setup/` | `src/setup/README.md` |
| `src/tools/` | `src/tools/README.md` |
| `src/workspace/` | `src/workspace/README.md` |
| `crates/ironclaw_webui/` | `crates/ironclaw_webui/CLAUDE.md` |
| `crates/ironclaw_reborn_identity/` | `crates/ironclaw_reborn_identity/CONTRACT.md` |
| `tests/integration/` | `tests/integration/CLAUDE.md` |
| `tests/support/reborn_parity_qa/` | `tests/support/reborn_parity_qa/CLAUDE.md` |
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

See `.env.example` for all environment variables. LLM backends (`nearai`, `openai`, `anthropic`, `ollama`, `openai_compatible`, `tinfoil`, `bedrock`) documented in `crates/ironclaw_llm/CLAUDE.md`.

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
