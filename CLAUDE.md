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

SKILL.md files extend the agent's prompt. Trust model: **Trusted** (user-placed, full tool access) vs **Installed** (registry, read-only tools). Selection: gating -> scoring -> budget -> attenuation. See `src/skills/` for implementation.

## Configuration

See `.env.example` for all environment variables. Key settings: `DATABASE_BACKEND`, `LLM_BACKEND` (nearai/openai/anthropic/ollama/openai_compatible/tinfoil), `GATEWAY_ENABLED`, `SANDBOX_ENABLED`, `SKILLS_ENABLED`, `ROUTINES_ENABLED`, `HEARTBEAT_ENABLED`.

## Workspace & Memory

Persistent memory with hybrid search (FTS + vector via RRF). Four tools: `memory_search`, `memory_write`, `memory_read`, `memory_tree`. Identity files (AGENTS.md, SOUL.md, USER.md, IDENTITY.md) injected into system prompt. See `src/workspace/README.md`.

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
