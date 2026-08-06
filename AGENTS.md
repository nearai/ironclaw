# Agent Rules

## Purpose and precedence

`AGENTS.md` is the canonical agent contract for this repository — the commands, hard invariants, and routing an agent cannot infer from the tree. It is not the full architecture specification: before changing a complex area, read the owning crate's `AGENTS.md`, then its `CONTRACT.md` or `README.md` when present; cross-crate behavior is specified under `docs/reborn/contracts/`. (`CLAUDE.md` files are Claude Code adapters and pointer stubs; content lives here and in the files this one names.)

All product work belongs in the Reborn workspace under `crates/`; the shipping binary is `ironclaw` from the `ironclaw` package in `crates/app/ironclaw_cli`. `crates/AGENTS.md` is the routing map into the ten crate families. The repo skills under `.claude/skills/` (`ironclaw-reborn-orientation`, `reborn-feature`, `ironclaw-reborn-architecture-review`, `ironclaw-reborn-testing`, `ironclaw-reborn-skill-maintainer`, `reborn-extension-surfaces`) are plain Markdown — read the `SKILL.md` directly if your harness does not load Claude skills.

## Build, run, debug

```bash
cargo fmt                                                       # format
cargo clippy --all --benches --tests --examples --all-features  # lint (zero warnings)
cargo test                                                      # unit + integration suites (Postgres legs self-provision testcontainers; skipped without Docker)
RUST_LOG=ironclaw=debug cargo run -p ironclaw -- serve          # run the serve binary (add tower_http=debug for HTTP logging)
```

The workspace-root `integration` feature is empty with zero consumers — a bare root `cargo test --features integration` adds nothing. Backend-heavy gated suites are crate-level (e.g. `cargo test -p ironclaw_hooks --features integration,test-support`). E2E suite: `tests/e2e/CLAUDE.md`.

**Cargo features are a last resort.** A feature is a second build of the workspace, compiled and tested forever. Add one only for a heavy optional dependency, a build shape that ships with it OFF, a CI lane selector, a dev-only seam (always named `test-support`), or a privilege boundary — and say which in the manifest comment. Deployment shape belongs in `DeploymentConfig` and `[storage]`, not `#[cfg]`. Full bar: `.claude/rules/cargo-features.md`.

## Discover code before changing it

For where-is, who-calls, data-flow, and impact questions, probe the codebase knowledge graph before text search: run `bash scripts/codebase-graph.sh status` once; if fresh and graph tools are connected, use them; otherwise fall back to `crates/AGENTS.md`, crate-local guidance, and targeted `rg`. Verify graph claims against live code before acting. Use `rg` directly for configuration, prose, and fixtures. `openwiki/` is generated prose — read-only, never hand-edit.

## Where work belongs

External surfaces normalize untrusted requests through product adapters or `ProductSurface`; thread/turn services establish durable conversation state; the scheduler and run executor invoke the canonical runner/driver and agent loop; capability execution crosses authorization, approvals, obligations, host-runtime mediation, and the selected runtime lane; durable typed events feed projections and transport streams — transports do not invent state. Verify a flow from live symbols:

```bash
rg -n "SessionThreadService|TurnCoordinator|TurnRunScheduler|RebornTurnRunExecutor|CanonicalAgentLoopExecutor|CapabilityHost" crates
```

Crates live under a family directory (`crates/<family>/ironclaw_*`); enumerate them with `python3 scripts/ci/lib/crate_tree.py .` rather than assuming a fixed depth. Stable ownership decisions:

- Neutral authority vocabulary belongs in `ironclaw_host_api`; execution does not.
- Filesystem mounts/CAS belong in `ironclaw_filesystem`; record grammar in the domain crate.
- Durable events, projections, and transport streams are separate contracts.
- Authorization, approvals, resources, obligations, dispatch, and runtime lanes remain separate stages.
- `ironclaw_assistant` owns product-facing orchestration and `ProductSurface`; composition wires dependencies; WebUI owns HTTP/transport and frontend presentation.
- Provider-neutral model contracts and provider implementations belong in `ironclaw_llm`; wrappers delegate the complete provider trait.
- Declarative extension metadata belongs in `ironclaw_extension_registry`; execution belongs in runtime lanes and host mediation.
- Safety scanning is `ironclaw_safety`; skills are `ironclaw_skills`; persistent memory is `ironclaw_memory` (model tools `ironclaw.memory.*`). Always import from the owning crate.

The composition root assembles dependencies; it does not own domain policy — module-specific initialization stays behind factories or builders in the owning crate. If adding a dependency would point from a lower neutral crate into product or composition, stop and run `cargo test -p ironclaw_architecture_tests` first.

Subagent spawn creates and wires child runs only; planning, execution, capability calls, checkpointing, gates, retries, and completion continue through the existing runner/driver/executor path.

Host-trusted trigger ingress is sealed by trigger-worker-owned request minting and private conversation-owned trusted construction. Product adapters, product workflow, first-party capabilities, and host-runtime handlers use untrusted inbound requests and must not mint `TrustedInboundTurnRequest` or call trusted trigger submitter factories.

## Module Specs

When modifying a module with a spec, read the spec first. Code follows spec; spec is the tiebreaker.

| Module | Spec |
|--------|------|
| `crates/domains/ironclaw_llm/` | `crates/domains/ironclaw_llm/CONTRACT.md` |
| `crates/substrates/ironclaw_filesystem/` | `crates/substrates/ironclaw_filesystem/CONTRACT.md` |
| `crates/product/ironclaw_webui/` | `crates/product/ironclaw_webui/CONTRACT.md` |
| `crates/app/ironclaw_composition/` | `crates/app/ironclaw_composition/CONTRACT.md` |
| `crates/domains/ironclaw_identity/` | `crates/domains/ironclaw_identity/CONTRACT.md` |
| `crates/kernel/ironclaw_trust/` | `crates/kernel/ironclaw_trust/CONTRACT.md` |
| `tests/` (scenario coverage map) | `tests/CLAUDE.md` |
| `tests/integration/` | `tests/integration/CLAUDE.md` |
| `tests/support/reborn_parity_qa/` | `tests/support/reborn_parity_qa/CLAUDE.md` |
| `tests/e2e/` | `tests/e2e/CLAUDE.md` |

## Coding and contract rules

- No `.unwrap()` or `.expect()` in production code (tests are fine); propagate errors with context — `.map_err(|e| SomeError::Variant { reason: e.to_string() })?` — and use `thiserror` for error types in `error.rs`.
- Keep clippy clean with zero warnings. Prefer `crate::` imports for cross-module references.
- Use strong types and enums for known domain shapes; raw strings belong at external boundaries. Shared types live with the contract owner — no mirror DTOs, and `ironclaw_common` is not a dumping ground.
- No `pub use` re-exports unless exposing to downstream consumers.
- **Prompt templates live in files, not Rust code**: multi-line prompt strings go in a `prompts/*.md` file inside the crate that owns the behavior, loaded via `include_str!()` (`ls -d crates/*/*/prompts crates/extensions/packages/*/prompts` lists the owners). Single-line format strings are fine inline.
- Preserve existing defaults unless the task explicitly changes them.
- All I/O is async with tokio; use `Arc<T>` for shared state.

## Testing discipline

1. **Test-first.** Every feature and fix starts in the tests — pin the behavior, watch it fail for the right reason, then change the implementation. Every fix ships with a regression test.
2. **Consolidate, don't proliferate.** Extend the test that already exercises the path; add a new test only for a genuinely distinct scenario.
3. **Integration-first.** Production-wired behavior ships with a test in `tests/integration/`, driven through the harness and asserting at a seam — never `wait_for_status(Completed)` alone. Crate tier is the fallback only when that tier cannot reach the path (say why in the PR).
4. **Test through the caller, not just the helper.** When a helper gates a side effect, unit-testing the helper alone is not regression coverage — drive the call site at the integration tier or higher, and make mocks capture every argument the production caller passes.

Full rules and tiers: `.claude/rules/testing.md`; authoring guides: `tests/integration/CLAUDE.md`, `tests/e2e/CLAUDE.md`. Select tiers with `docs/internal/testing-playbook.md`, and complete the `Test Strategy` section of `.github/pull_request_template.md` with evidence or `Not applicable: <reason>` per tier.

## Persistence and configuration

New persistence uses `RootFilesystem`/`ScopedFilesystem` and the mount catalog owned by `ironclaw_filesystem` (spec above); composition chooses concrete backends (PostgreSQL, libSQL, local filesystem) by profile. Domain stores are thin typed wrappers and never branch on backend; keep dual-backend parity via shared conformance suites (`.claude/rules/database.md`). Read-modify-write uses the shared bounded CAS helper, never a process-local mutex held across backend I/O.

Keep bootstrap configuration, persisted settings, and encrypted secrets as separate layers; preserve configuration precedence, secret-mediated provider resolution, and fail-closed startup. Environment variables are documented in `.env.example`; LLM backends in the llm spec (`LlmBackendKind` in `crates/domains/ironclaw_llm/src/config.rs` is the source of truth).

## Security and runtime invariants

- Treat every listener, route, product adapter, runtime lane, container, and external service as untrusted until a typed boundary establishes otherwise.
- Do not weaken authentication, origin checks, body limits, rate limits, allowlists, approval leases, secret mediation, or redaction guarantees.
- External HTTP goes through `ironclaw_network`; credentials remain host-side and are injected only through mediated runtime services.
- New ingress must validate and bound the original payload before persistence, prompt construction, credential injection, or dispatch.
- Authorization, approval, reservation, dispatch, and execution are distinct stages. Do not bypass or collapse them — product/WebUI handlers, triggers, channels, and agent callers go through `ProductSurface` and the capability contracts, never around them to mutate stores directly.
- Session, thread, turn, and run identities are typed and must not be re-derived from display strings or transport metadata.
- **LLM data is never deleted.** Context, reasoning, tool calls, messages, events, steps — mark with timestamps and make filterable, but always retain. In-memory maps are caches; the database is the source of truth. "Cleanup" means evicting caches, never deleting rows.
- Never commit secrets or PII.

## Capabilities, extensions, and lifecycle

- Core host behavior uses typed built-in capabilities behind the same mediated host surface as other execution.
- Sandboxed extension execution belongs in WASM or a runtime lane; external server integrations belong behind MCP and the network boundary.
- Discovery is side-effect-free. Installation, credential binding, activation, execution, deactivation, and removal are explicit lifecycle transitions.
- Capability failures the model or user can correct are model-visible outcomes; host errors are reserved for failures that end the run.
- Side-effecting success requires durable or provider-issued evidence plus read-back verification; if read-back is impossible, report explicitly unverified rather than completed.

### Extension/Auth Invariants

The top-level product object is always an **extension**; a channel is one capability surface an extension's manifest declares (`tool` / `channel` / `auth` — `ironclaw_extension_contracts::surface::CapabilitySurfaceKind`), and runtime (`wasm` / `mcp` / `first_party`) is implementation, never taxonomy. `ExtensionId` is the product identity (`slack`, `github`, `gmail`); `VendorId` (manifest field `vendor`) is the credential-authority namespace and may back several extensions (`google` backs gmail + drive + calendar). There is no separate channel registry and no extension `kind` wire string — `crates/app/ironclaw_architecture_tests/tests/reborn_retired_taxonomy.rs` pins the retired vocabulary at zero.

Two identities must never be conflated (newtypes in `crates/contracts/ironclaw_common/src/identity.rs`; identity model `crates/domains/ironclaw_identity/CONTRACT.md`; OAuth transport `crates/domains/ironclaw_auth`):

- `credential_name` — backend secret identity (storage, injection, gate resume), e.g. `telegram_bot_token`, `google_oauth_token`.
- `extension_name` — user-facing installed extension/channel identity (setup routing, UI), e.g. `telegram`, `gmail`.

Never route setup/configure UI from `credential_name`; chat and Settings use the same setup path; generic auth-card UI is only for non-extension credential prompts or pure OAuth launches; resolve `extension_name` once in shared backend logic and carry it through the wire contract instead of re-deriving it per layer or adding frontend-only fallbacks.

Adding a channel means adding one capability surface of an extension — a `[channel]` section in the `reborn.extension_manifest.v3` manifest plus a `ChannelAdapter` (`crates/contracts/ironclaw_extension_contracts/src/channel_adapter.rs`), wired through `RebornHostBindings::with_channel_extension_bindings` (`crates/app/ironclaw_composition/src/input.rs`) — never per-channel host code. Start from the `reborn-extension-surfaces` skill; the worked example is `crates/extensions/packages/slack/`; family rules in `crates/extensions/AGENTS.md`.

## Project structure

```
crates/                     # all production code, by family (crates/AGENTS.md is the map)
├── app/                    # ironclaw_cli (binary `ironclaw`), ironclaw_composition, ironclaw_config, ironclaw_architecture_tests
├── contracts/              # ironclaw_host_api, ironclaw_common, ironclaw_extension_contracts, ironclaw_product_contracts, …
├── domains/                # ironclaw_llm, ironclaw_skills, ironclaw_threads, ironclaw_auth, ironclaw_memory, …
├── events/                 # ironclaw_event_log / _projections / _store / _streams
├── extensions/             # ironclaw_extension_host/_manager/_registry/_support + packages/ (slack, telegram, …)
├── kernel/                 # ironclaw_turns, ironclaw_capabilities, ironclaw_approvals, ironclaw_host_runtime, …
├── lanes/                  # ironclaw_wasm, ironclaw_sandbox, ironclaw_mcp
├── loop/                   # ironclaw_agent_loop, ironclaw_turn_runner, ironclaw_loop_host, ironclaw_hooks
├── product/                # ironclaw_webui (SPA in frontend/), ironclaw_assistant, …
└── substrates/             # ironclaw_filesystem, ironclaw_safety, ironclaw_network, ironclaw_secrets, …

tests/                      # root-package integration suite, parity/QA, support, e2e
```

The workspace root (`Cargo.toml`, package `ironclaw_integration_tests`) hosts only the integration test suite; the one workspace `exclude` is `tools/ironclaw_silk_decoder`.

## Change discipline, and before finishing

- Keep changes scoped; preserve unrelated work in dirty worktrees; avoid generated-file churn. Security, persistence-schema, runtime, worker, CI, and secrets changes need explicit rollback/compatibility review.
- Run the narrowest meaningful checks, plus `cargo test -p ironclaw_architecture_tests` when dependency edges, layer keys, crate placement, or test-pinned guidance files change.
- Search changed production files for `.unwrap()`/`.expect()`, suspicious byte slicing, hardcoded temporary paths, and lost error causes.
- When a trait changes, enumerate all implementations, decorators, adapters, and test doubles; when a pattern bug is fixed, search `crates/` for sibling instances.
- After moves/renames, search agent guidance, contracts, docs, tests, scripts, manifests, and frontend imports for old paths.
- Update the owning contract/docs when behavior changes; the PR title/body must describe every layer in the diff and note compatibility, rollback, and follow-up risks.
