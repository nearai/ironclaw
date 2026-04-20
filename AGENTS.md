# Agent Rules

## Purpose and Precedence

- `AGENTS.md` is the quick-start contract for coding agents. It is not the full architecture spec.
- Read the relevant subsystem spec before changing a complex area. When a repo spec exists, treat it as authoritative.
Start with these deeper docs as needed:
- `CLAUDE.md`
- `src/agent/CLAUDE.md`
- `src/channels/web/CLAUDE.md`
- `src/db/CLAUDE.md`
- `src/llm/CLAUDE.md`
- `src/setup/README.md`
- `src/tools/README.md`
- `src/workspace/README.md`
- `src/NETWORK_SECURITY.md`
- `tests/e2e/CLAUDE.md`
- In this repo, `ic/` contains the core daemon; however, the product name is LunarWing.
- In this repo, the path `ic/customic` is not used at all. It will be removed at some point.

## Architecture Mental Model

- Channels normalize external input into `IncomingMessage`; `ChannelManager` merges all active channel streams.
- `Agent` owns session/thread/turn handling, submission parsing, the LLM/tool loop, approvals, routines, and background runtime behavior.
- `AppBuilder` is the composition root that wires database, secrets, LLMs, tools, workspace, extensions, skills, hooks, and cost controls before the agent starts.
- The web gateway is a browser-facing API/UI layered on top of the same agent/session/tool systems, not a separate product path.

## Where to Work

- Agent/runtime behavior: `src/agent/`
- Web gateway/API/SSE/WebSocket: `src/channels/web/`
- Persistence and DB abstractions: `src/db/`
- Setup/onboarding/configuration flow: `src/setup/`
- LLM providers and routing: `src/llm/`
- Workspace, memory, embeddings, search: `src/workspace/`
- Extensions, tools, channels, MCP, WASM: `src/extensions/`, `src/tools/`, `src/channels/`
- OpenClaw port staging work: `openclaw-ports/`. For OpenClaw port tasks, keep edits inside `openclaw-ports/` unless the user explicitly approves touching core IronClaw files.

## Ownership and Composition Rules

- Keep `src/main.rs` and `src/app.rs` orchestration-focused. Do not move module-owned logic into entrypoints.
- Module-specific initialization should live in the owning module behind a public factory/helper, not be reimplemented ad hoc.
- Keep feature-flag branching inside the module that owns the abstraction whenever possible.
- Prefer extending existing traits and registries over hardcoding one-off integration paths.

## Repo-Wide Coding Rules

- Avoid `.unwrap()` and `.expect()` in production; prefer proper error handling. They are fine in tests, and in production only for truly infallible invariants (e.g., literals/regexes) with a safety comment.
- Keep clippy clean with zero warnings.
- Prefer `crate::` imports for cross-module references.
- Use strong types and enums over stringly-typed control flow when the shape is known.

## Database, Setup, and Config Rules

- New persistence behavior must support both PostgreSQL and libSQL.
- Add new DB operations to the shared DB trait first, then implement both backends.
- Treat bootstrap config, DB-backed settings, and encrypted secrets as distinct layers; do not collapse them casually.
- If onboarding or setup behavior changes, update `src/setup/README.md` in the same branch.
- Do not break config precedence, bootstrap env loading, DB-backed config reload, or post-secrets LLM re-resolution.

## Security and Runtime Invariants

- Review any change touching listeners, routes, auth, secrets, sandboxing, approvals, or outbound HTTP with a security mindset.
- Do not weaken bearer-token auth, webhook auth, CORS/origin checks, body limits, rate limits, allowlists, or secret-handling guarantees.
- Treat Docker containers and external services as untrusted.
- Session/thread/turn state matters. Submission parsing happens before normal chat handling.
- Skills are selected deterministically. Tool approval and auth flows are special paths and must not be mixed into normal chat history carelessly.
- Persistent memory is the workspace system, not just transcript storage; preserve file-like semantics, chunking/search behavior, and identity/system-prompt loading.

## Tools, Channels, and Extensions

- Use a built-in Rust tool for core internal capabilities tightly coupled to the runtime.
- Use WASM tools or WASM channels for sandboxed extensions and plugin-style integrations.
- Use MCP for external server integrations when the capability belongs outside the main binary.
- Preserve extension lifecycle expectations: install, authenticate/configure, activate, remove.

## Local XMPP and Service Operations

- Treat systemd unit environment values as secret-bearing. Do not paste passwords, bearer tokens, or webhook secrets into user-facing output; summarize or redact them.
- `xmpp-bridge.service` is coupled to `ironclaw.service` with `PartOf=ironclaw.service`, so IronClaw restarts can also restart the bridge. Do not assume the bridge caused an IronClaw stop just because both units restarted together.
- Use `scripts/xmpp-rate-limit.sh` for live XMPP outbound rate-limit changes. It requires `XMPP_BRIDGE_TOKEN`; `status`, `set <n>`, `off`, and `reset` are the main commands.
- Use `scripts/xmpp-configure.sh` for bridge room/configuration checks and configure calls when working with the existing XMPP bridge API.
- The local service watchdog files are `scripts/ironclaw-watchdog.sh`, `scripts/install-ironclaw-watchdog.sh`, `systemd/ironclaw-watchdog.service`, and `systemd/ironclaw-watchdog.timer`. The installed timer checks `ironclaw.service` hourly and logs to `/var/log/ironclaw-watchdog.log` plus journald.
- Prefer read-only diagnostics first for service issues: `systemctl status`, `systemctl show`, `journalctl`, and bridge status endpoints. Only restart services after identifying the unit state or when the user explicitly asks.

## Docs, Parity, and Testing

- If behavior changes, update the relevant docs/specs in the same branch.
- If you change implementation status for any feature tracked in `FEATURE_PARITY.md`, update that file in the same branch.
- Do not open a PR that changes feature behavior without checking `FEATURE_PARITY.md` for needed status updates (`❌`, `🚧`, `✅`, notes, and priorities).
- Add the narrowest tests that validate the change: unit tests for local logic, integration tests for runtime/DB/routing behavior, and E2E or trace coverage for gateway, approvals, extensions, or other user-visible flows.

## Risk and Change Discipline

- Keep changes scoped; avoid broad refactors unless the task truly requires them.
- Security, database schema, runtime, worker, CI, and secrets changes are high-risk. Call out rollback risks, compatibility concerns, and hidden side effects.
- Preserve existing defaults unless the task explicitly changes them.
- Avoid unrelated file churn and generated-file edits unless required.
- Respect a dirty worktree and never revert user changes you did not make.

## Before Finishing

- Confirm whether behavior changes require updates to `FEATURE_PARITY.md`, specs, API docs, or `CHANGELOG.md`.
- Run the most targeted tests/checks that cover the change.
- Re-check security-sensitive paths when touching auth, secrets, network listeners, sandboxing, or approvals.
- Keep the final diff scoped to the task.
