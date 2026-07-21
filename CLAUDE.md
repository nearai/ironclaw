# IronClaw Development Guide

**IronClaw** is a secure personal AI assistant — user-first security, self-expanding tools, defense in depth, multi-channel access with proactive background execution.

## Code Discovery — Query the Knowledge Graph First

This repo can be indexed into a **codebase knowledge graph** (the `codebase-memory` MCP server) over `crates/`. For any *where-is / who-calls / how-does-data-flow / what-does-this-touch* question, **probe the graph before reaching for `Grep`** — text search cannot see cross-crate call chains, and this codebase's real cost is cross-crate (a feature crosses `product_workflow → composition → webui_v2 → runtime → frontend`).

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

## Where to Build — the Reborn stack in `crates/`

**All work lives in the Reborn stack under `crates/`.** The v1 `src/` monolith
(package `ironclaw_legacy`) was deleted under Tier B
(`docs/plans/2026-07-02-reborn-internal-module-refactor.md` §8) — there is no
longer a v1 codebase to disambiguate from. A Reborn feature crosses
`product_workflow → composition → webui_v2 → runtime/serve → frontend`; the
binary entry point is `crates/ironclaw_reborn_cli` (binary name `ironclaw`).
Start from the `reborn-feature` skill — it maps those layers so you wire a
feature in one pass instead of layer-by-layer. The workspace root
(`Cargo.toml`, package `ironclaw_reborn_integration_tests`) now hosts only the
Reborn integration test suite (`tests/integration/*`); it has no lib/bin of its
own.

## Build & Test

```bash
cargo fmt                                                    # format
cargo clippy --all --benches --tests --examples --all-features  # lint (zero warnings)
cargo test                                                   # unit tests
cargo test --features integration                            # + PostgreSQL tests
RUST_LOG=ironclaw=debug cargo run -p ironclaw -- serve       # run the Reborn serve binary with logging
```

E2E tests: see `tests/e2e/CLAUDE.md`.

### Cargo features are a last resort

A feature is a second build of the workspace that must be compiled,
linted, and tested forever. Add one only for a heavy optional dependency,
a build shape something actually ships with it OFF, a CI lane selector, a
dev-only seam (always named `test-support`), or a privilege boundary — and
say which in the manifest comment. Deployment shape belongs in
`DeploymentConfig` and `[storage]`, not `#[cfg]`. Full bar, the
delete-checklist, and the feature-gated-dead-code trap:
`.claude/rules/cargo-features.md`.

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

The `credential_name` / `extension_name` newtypes live in
`crates/ironclaw_common/src/identity.rs` (see `.claude/rules/types.md`). The v1
resolver/auth-flow ownership (`src/auth/extension.rs`, the web `pending_auth`
path, the `ironclaw_gateway` onboarding JS) was deleted under Tier B; the Reborn
identity/product-auth model lives in the Reborn crates
(`crates/ironclaw_reborn_identity/CONTRACT.md`, `crates/ironclaw_oauth`,
`crates/ironclaw_auth`) and its WebUI onboarding in
`crates/ironclaw_webui/frontend`.

Key traits for extensibility: `Channel`, `Tool`, `LlmProvider`, `SuccessEvaluator`, `EmbeddingProvider`, `NetworkPolicyDecider`, `Hook`, `Observer`, `Tunnel`.

All I/O is async with tokio. Use `Arc<T>` for shared state, `RwLock` for concurrent access.

**LLM data is never deleted.** All LLM output — context fed to the model, reasoning, tool calls, messages, events, steps — is the most valuable data in the system. Never strip, truncate, or delete it from the database. Mark with timestamps, make filterable, but always retain. In-memory HashMaps are caches; the database (via Workspace) is the source of truth. "Cleanup" means evicting from in-memory caches, never deleting database rows.

## Extracted Crates

Safety logic lives in `crates/ironclaw_safety/`, skills in `crates/ironclaw_skills/`, multi-provider LLM integration in `crates/ironclaw_llm/`. **Import directly from the extracted crate** (e.g. `use ironclaw_safety::SafetyLayer`, `use ironclaw_skills::SkillRegistry`, `use ironclaw_llm::{LlmProvider, LlmError}`). These crates are the sole home of their types now that the v1 `src/` monolith (which formerly re-exported some of them) has been deleted under Tier B — always import from the owning crate.

## Project Structure

All production code lives under `crates/` (the Reborn stack). The v1 `src/`
monolith and the `ironclaw_gateway` / `ironclaw_tui` crates were deleted under
Tier B. For where a symbol or subsystem lives, query the codebase knowledge
graph (see "Code Discovery" above) or read the relevant crate's `CLAUDE.md` /
`AGENTS.md`; `crates/AGENTS.md` is the crate-level map. The reborn-feature
flow crosses `product_workflow → composition → webui_v2 → runtime/serve →
frontend` (binary `ironclaw` in `crates/ironclaw_reborn_cli`).

```
crates/                     # all production code (see crates/AGENTS.md for the full map)
├── ironclaw_reborn_cli/    # binary entry point (binary name `ironclaw`)
├── ironclaw_reborn_composition/  # wires storage/runtime services by profile
├── ironclaw_product_workflow/    # product-facing workflow surface
├── ironclaw_runner/ ironclaw_turns/ ironclaw_agent_loop/  # turn runtime + agent loop
├── ironclaw_webui/         # WebChat v2 SPA (frontend/) + serve wiring
├── ironclaw_filesystem/    # RootFilesystem mount catalog (persistence plane)
├── ironclaw_llm/ ironclaw_safety/ ironclaw_skills/  # extracted subsystems
├── ironclaw_reborn_migration/    # v1 → Reborn state migration tool
└── ...                     # domain crates (threads, secrets, oauth, triggers, …)

tests/
├── integration/            # Reborn in-process integration tests (see tests/integration/CLAUDE.md)
├── reborn_*.rs             # Reborn parity/QA tests
├── support/                # shared test support (trace_llm, mocks)
└── e2e/                    # Python/Playwright E2E scenarios (see tests/e2e/CLAUDE.md)
```

## Database

Reborn persistence goes through the `RootFilesystem` mount catalog
(`crates/ironclaw_filesystem`); composition chooses concrete backends
(PostgreSQL, libSQL, local filesystem) by profile. Domain crates own record
schemas and never branch on backend. When a domain supports multiple durable
backends, keep behavioral parity via a shared conformance suite. See
`crates/ironclaw_filesystem/CLAUDE.md` and `.claude/rules/database.md`.

## Module Specs

When modifying a module with a spec, read the spec first. Code follows spec; spec is the tiebreaker.

**Module-owned initialization:** Module-specific initialization logic (storage connection, transport creation, service wiring) must live in the owning crate as a public factory function — not reconstructed at the composition/route layer. Composition (`ironclaw_reborn_composition`) orchestrates calls to crate factories and chooses concrete backends by profile.

| Module | Spec |
|--------|------|
| `crates/ironclaw_llm/` | `crates/ironclaw_llm/CLAUDE.md` |
| `crates/ironclaw_embeddings/` | `crates/ironclaw_embeddings/AGENTS.md` |
| `crates/ironclaw_filesystem/` | `crates/ironclaw_filesystem/CLAUDE.md` |
| `crates/ironclaw_webui/` | `crates/ironclaw_webui/CLAUDE.md` |
| `crates/ironclaw_reborn_composition/` | `crates/ironclaw_reborn_composition/CLAUDE.md` |
| `crates/ironclaw_reborn_identity/` | `crates/ironclaw_reborn_identity/CONTRACT.md` |
| `crates/ironclaw_reborn_migration/` | `crates/ironclaw_reborn_migration/CLAUDE.md` |
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

Reborn channels are WASM modules (source under `channels-src/`, built via
`scripts/build-wasm-extensions.sh`) plus the host-side channel crates
(`ironclaw_channel_host`, `ironclaw_channel_delivery`, and per-channel
extensions such as `ironclaw_telegram_extension`). Wire the channel through
composition (`ironclaw_reborn_composition`) rather than reconstructing runtime
state at the route layer. See `crates/AGENTS.md` for the channel crate map.

## Everything Goes Through Capability Dispatch

**Core principle**: all actions originating from product/WebUI handlers,
scheduled triggers, channels, or any other non-agent caller route through the
**same capability/tool dispatch path** the agent loop uses — they do not reach
around it to mutate stores directly. This gives every caller-initiated mutation
the same audit trail, safety pipeline (param validation, redaction, output
sanitization), and authorization/approval surface as agent-initiated tool
calls.

This invariant is enforced structurally in the product-workflow and
composition crates and by `cargo test -p ironclaw_architecture` (dependency and
composition boundaries), not by a grep-based pre-commit check. See
`.claude/rules/tools.md`.

## Workspace & Memory

Persistent memory with hybrid search lives in `crates/ironclaw_memory`
(`memory_search`, `memory_write`, `memory_read`, `memory_tree`), persisted
through the `RootFilesystem` mount catalog. Identity files (AGENTS.md, SOUL.md,
USER.md, IDENTITY.md) are injected into the system prompt. (The v1 heartbeat
proactive-execution loop has no Reborn equivalent yet — see issue #6369.)

## Debugging

```bash
RUST_LOG=ironclaw=trace cargo run -p ironclaw           # verbose
RUST_LOG=ironclaw=debug,tower_http=debug cargo run -p ironclaw  # + HTTP request logging
```

## Current Limitations

1. Integration tests need testcontainers for PostgreSQL
2. MCP: no streaming support; stdio/HTTP/Unix transports all use request-response
3. WIT bindgen: auto-extract tool schema from WASM is stubbed
4. No tool versioning or rollback
5. Observability: only `log` and `noop` backends (no OpenTelemetry)
