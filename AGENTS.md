# Agent Rules

## Purpose and Precedence
- `AGENTS.md` is quick-start; read subsystem specs for complex changes.
- Deeper docs: `CLAUDE.md`, `src/agent/CLAUDE.md`, `src/channels/web/CLAUDE.md`, `src/db/CLAUDE.md`, `src/llm/CLAUDE.md`, `src/setup/README.md`, `src/tools/README.md`, `src/workspace/README.md`, `src/NETWORK_SECURITY.md`, `tests/e2e/CLAUDE.md`

## Architecture Mental Model
- Channels normalize input → `IncomingMessage`; `ChannelManager` merges streams.
- `Agent` owns session/thread/turn, LLM/tool loop, approvals, routines.
- `AppBuilder` wires DB, secrets, LLMs, tools, workspace, extensions, hooks.
- Web gateway layers on agent/session/tool systems.

## Where to Work
- Agent/runtime: `src/agent/`
- Web gateway/API/SSE/WebSocket: `src/channels/web/`
- Persistence: `src/db/`
- Setup/onboarding: `src/setup/`
- LLM providers: `src/llm/`
- Workspace/memory/embeddings: `src/workspace/`
- Extensions/tools/channels/MCP/WASM: `src/extensions/`, `src/tools/`, `src/channels/`

## Ownership and Composition
- Keep `main.rs`/`app.rs` orchestration-focused.
- Module-specific init lives in owning module.
- Feature-flag branching inside owning module.

## Coding Rules
- No `.unwrap()`/`.expect()` in production (tests OK, infallible literals OK with comment).
- Clippy clean, zero warnings.
- Prefer `crate::` imports.
- Strong types/enums over stringly-typed control flow.

## Database and Config
- New persistence must support PostgreSQL and libSQL.
- Add to shared DB trait first, implement both backends.
- Bootstrap config, DB-backed settings, encrypted secrets are distinct layers.
- Update `src/setup/README.md` if onboarding changes.

## Security and Runtime
- Review listeners, routes, auth, secrets, sandboxing, approvals, outbound HTTP.
- Do not weaken bearer-token auth, webhook auth, CORS, body limits, rate limits, allowlists.
- Docker/external services are untrusted.
- Submission parsing precedes chat handling.
- Skills selected deterministically; tool approval/auth are special paths.
- Persistent memory = workspace system (file-like semantics, chunking/search).

## Tools, Channels, Extensions
- Built-in Rust tool: core internal capabilities.
- WASM tools/channels: sandboxed extensions, plugins.
- MCP: external server integrations.
- Preserve extension lifecycle: install → auth/config → activate → remove.

## Docs, Parity, Testing
- Update docs/specs with behavior changes.
- Update `FEATURE_PARITY.md` status (`❌`, `🚧`, `✅`) when implementation changes.
- Narrowest tests: unit (local), integration (runtime/DB/routing), E2E (gateway/approvals/extensions).

## Risk and Change Discipline
- Keep changes scoped; avoid broad refactors.
- High-risk: security, DB schema, runtime, worker, CI, secrets.
- Preserve existing defaults unless task explicitly changes them.
- No unrelated file churn.

## Before Finishing
- Check `FEATURE_PARITY.md`, specs, API docs, `CHANGELOG.md` for needed updates.
- Run targeted tests covering the change.
- Re-check security paths (auth, secrets, listeners, sandboxing, approvals).
- Keep diff scoped to task.
