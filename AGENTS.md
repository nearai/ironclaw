# Agent Rules

## Purpose and Precedence

- `AGENTS.md` is the quick-start map for coding agents. It is not the full architecture spec.
- Treat deeper subsystem docs as authoritative before changing complex areas:
  - Root guide: `CLAUDE.md`
  - Agent/runtime: `src/agent/CLAUDE.md`
  - Web gateway/API/SSE/WebSocket: `src/channels/web/CLAUDE.md`
  - Database backends: `src/db/CLAUDE.md`
  - LLM providers/routing: `src/llm/CLAUDE.md`
  - Setup/onboarding: `src/setup/README.md`
  - Tools/extensions: `src/tools/README.md`
  - Workspace/memory: `src/workspace/README.md`
  - Network/security: `src/NETWORK_SECURITY.md`
  - Browser E2E: `tests/e2e/CLAUDE.md`
  - Internal crates: `crates/*/CLAUDE.md` when present
- Prefer tracked files (`git ls-files`, `find`, `grep`) over local scratch dirs; ignore generated or operator-local output such as `target/`, `artifacts/`, `live-canary-*`, `worktrees/`, and `.kb/worktrees/`.

## Repository Map

- `src/` — root application, CLI, composition, agent loop, channels, DB, LLM, setup, tools, workspace, worker runtime.
- `crates/` — internal Cargo workspace crates (`ironclaw_engine`, `ironclaw_gateway`, `ironclaw_tui`, host/runtime/capability/resource crates).
- `channels-src/` — WASM channel extension crates (`discord`, `slack`, `telegram`, etc.), excluded from root workspace.
- `tools-src/` — WASM tool extension crates plus `tools-src/TOOLS.md`, excluded from root workspace.
- `tests/` — Rust integration tests, fixtures, snapshots, and Python/Playwright E2E in `tests/e2e/`.
- `migrations/` and `src/db/` — PostgreSQL migrations plus libSQL schema/migration code.
- `registry/`, `skills/`, `wit/` — bundled extension registries, skill definitions, and WIT interfaces.
- `docs/`, `CONTRIBUTING.md`, `FEATURE_PARITY.md` — user/operator docs, contribution rules, and tracked feature status.
- `scripts/`, `.github/workflows/`, `.config/nextest.toml` — local/CI validation, canaries, nextest timeout policy.
- `profiles/`, `deploy/`, `docker/`, `infra/runner/` — runtime profiles, deployment, containers, and runner images.

## Build, Test, and Run

- Setup: `./scripts/dev-setup.sh`
- Format: `cargo fmt`
- Lint: `cargo clippy --all --benches --tests --examples --all-features`
- Unit tests: `cargo test`
- PostgreSQL/integration tests: `cargo test --features integration`
- Run locally: `RUST_LOG=ironclaw=debug cargo run`
- Pre-review gate from `CONTRIBUTING.md`:
  - `cargo fmt --all -- --check`
  - `cargo clippy --all --benches --tests --examples --all-features -- -D warnings`
  - `cargo build`
  - `cargo test`
- Dependency changes: run `cargo deny check`.
- libSQL checks: `cargo check --no-default-features --features libsql`; both DBs: `cargo check --all-features`.
- Browser E2E: `cd tests/e2e && pip install -e . && playwright install chromium && pytest scenarios/ -v --timeout=120`.
- WASM extension build shape: `cargo build --manifest-path tools-src/<name>/Cargo.toml --target wasm32-wasip2 --release` or same under `channels-src/<name>/`.

## CI and Validation Signals

- `.github/workflows/code_style.yml` runs formatting, `cargo deny`, clippy matrices, gateway JS syntax, and boundary checks.
- `.github/workflows/test.yml` is the reusable core test workflow for PRs, merge queue, main, and nightly callers.
- `.github/workflows/e2e.yml` builds `--no-default-features --features libsql` and runs Playwright groups with `timeout-minutes: 90`.
- `.github/workflows/nightly-deep-ci.yml` reuses `test.yml` for deterministic deep checks and excludes Docker.
- `.config/nextest.toml` owns cargo-nextest slow-timeout overrides; update it when a test legitimately needs more time.

## Architecture Rules

- Keep `src/main.rs` and `src/app.rs` orchestration-focused. Put module-owned initialization behind public factories/helpers in the owning module.
- Channels normalize external input into `IncomingMessage`; `ChannelManager` merges active streams.
- `Agent` owns sessions, threads, turns, submission parsing, LLM/tool loop, approvals, routines, and background runtime behavior.
- `AppBuilder` wires DB, secrets, LLMs, tools, workspace, extensions, skills, hooks, and cost controls before the agent starts.
- The web gateway is a browser-facing layer over the same agent/session/tool systems, not a separate product path.
- Prefer extending traits/registries over hardcoding one-off integration paths.
- Use built-in Rust tools for core runtime capabilities, WASM tools/channels for sandboxed extensions, and MCP for external server integrations.

## High-Risk Areas and Invariants

- Production code: avoid `.unwrap()`/`.expect()`; use typed errors (`thiserror`) and contextual `map_err`.
- Prefer `crate::` imports for cross-module references; use strong types/enums over stringly control flow.
- Persistence: add DB operations to the shared DB trait first, then implement PostgreSQL and libSQL. New persistence behavior must support both backends.
- Migrations: base new versions on `origin/staging`/main state; do not reuse deployed migration numbers.
- Setup/config: preserve bootstrap config, DB-backed settings, encrypted secrets, config precedence, env loading, and post-secrets LLM re-resolution.
- Security-sensitive changes (listeners, routes, auth, secrets, sandboxing, approvals, outbound HTTP) must preserve bearer auth, webhook auth, CORS/origin checks, body limits, rate limits, allowlists, and secret-handling guarantees.
- Web gateway: route composition lives in `src/channels/web/platform/router.rs`; other `platform/*` modules must stay handler-agnostic per `scripts/check_gateway_boundaries.py`.
- Web identity: extension/setup routes use `ExtensionName`; do not expose `CredentialName` in web DTOs/handlers.
- Session/thread/turn state is critical. Submission parsing happens before normal chat handling; auth and approval flows are special paths.
- Workspace memory is file-like persistent storage with chunking/search semantics; do not treat it as transcript-only storage.

## Docs, Parity, and Testing

- If behavior changes, update relevant docs/specs and check `FEATURE_PARITY.md` in the same branch.
- If onboarding/setup changes, update `src/setup/README.md`.
- If channels, tools, or user-facing capabilities change, update matching docs under `docs/` and/or `tools-src/`/`channels-src/` READMEs.
- Add the narrowest meaningful tests, but test through the caller when a helper gates side effects (HTTP, DB writes, OAuth, UI mutation, tool execution).
- For E2E tests, use `tests/e2e/helpers.py` selectors/constants; raw SSE assertions belong in `aiohttp`, not Playwright.
- Keep changes scoped; avoid unrelated churn and generated-file edits. Respect dirty worktrees and never revert user changes you did not make.
