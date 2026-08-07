# Agent Rules

## Purpose and precedence

`AGENTS.md` is the quick-start contract for coding agents. It is not the full
architecture specification. Before changing a complex area, read the owning
crate's `AGENTS.md`, then its `CLAUDE.md`, `CONTRACT.md`, or `README.md` when
present. Cross-crate behavior is specified under `docs/reborn/contracts/`.

All product work belongs in the Reborn workspace under `crates/`. The shipping
binary is `ironclaw` from the `ironclaw` package in
`crates/app/ironclaw_cli`. Start with:

- `.claude/skills/ironclaw-reborn-orientation/SKILL.md` for ownership and flow.
- `.claude/skills/reborn-feature/SKILL.md` for cross-layer product work.
- `.claude/skills/ironclaw-reborn-architecture-review/SKILL.md` for boundaries.
- `.claude/skills/ironclaw-reborn-testing/SKILL.md` for test tiers and seams.
- `.claude/skills/ironclaw-reborn-skill-maintainer/SKILL.md` before editing guidance.

These are plain Markdown and must be read directly by agents whose harness does
not load Claude skills.

## Discover code before changing it

For where-is, who-calls, data-flow, and impact questions, probe the codebase
knowledge graph before text search:

1. Run `bash scripts/codebase-graph.sh status` once.
2. If fresh and the graph tools are connected, use symbol search, call tracing,
   data-flow tracing, and architecture queries as appropriate.
3. If missing, stale, or unavailable, fall back immediately to `crates/AGENTS.md`,
   the Reborn orientation skill, crate-local guidance, and targeted `rg`.
4. Verify graph claims against live code before acting.

Use `rg` directly for configuration, prose, fixtures, and other non-code data.
Narrative docs under `openwiki/` are generated and read-only.

## Reborn architecture mental model

External surfaces normalize untrusted requests through product adapters or
`ProductSurface`. Thread and turn services establish durable conversation
state. The scheduler and Reborn run executor invoke the canonical runner/driver
and agent loop. Capability execution then crosses authorization, approvals,
obligations, host-runtime mediation, and the selected runtime lane. Durable typed
events feed projections and transport streams; transports do not invent state.

Use `crates/AGENTS.md` to locate the owning crate, and regenerate any detailed
flow from live symbols. A useful verification recipe is:

```bash
rg -n "SessionThreadService|TurnCoordinator|TurnRunScheduler|RebornTurnRunExecutor|CanonicalAgentLoopExecutor|CapabilityHost" crates
```

The composition root assembles dependencies; it does not own domain policy.
Module-specific initialization stays behind factories or builders in the owning
crate. Keep feature branching with the abstraction owner and prefer existing
typed ports and registries over one-off integration paths.

## Where Reborn work belongs

Use `crates/AGENTS.md` as the routing index, then verify ownership from live
dependencies and public contracts:

```bash
rg -n "CONCEPT_OR_TRAIT_OR_TYPE" crates
rg -n "ironclaw_CANDIDATE" --glob '**/Cargo.toml' crates Cargo.toml
# Crates live under a family directory (crates/<family>/ironclaw_*, PROPOSAL §5),
# so resolve the crate by name rather than assuming a fixed depth:
find "$(python3 scripts/ci/lib/crate_tree.py . | grep -E '/ironclaw_CANDIDATE$')" -maxdepth 2 \
  \( -name AGENTS.md -o -name CLAUDE.md -o -name CONTRACT.md -o -name README.md \)
```

Stable ownership decisions:

- Neutral authority vocabulary belongs in `ironclaw_host_api`; execution does
  not.
- Filesystem mounts/CAS belong in `ironclaw_filesystem`; domain record grammar
  belongs in the domain crate.
- Durable events, projections, and transport streams are separate contracts.
- Authorization, approvals, resources, obligations, dispatch, and runtime lanes
  remain separate stages.
- `ironclaw_assistant` owns product-facing orchestration and `ProductSurface`
  descriptors; composition wires dependencies; WebUI owns HTTP/transport and
  frontend presentation.
- Provider-neutral model contracts and provider implementations belong in
  `ironclaw_llm`; wrappers must delegate the complete provider trait.
- Declarative extension metadata belongs in `ironclaw_extension_registry`; execution
  belongs in runtime lanes and host mediation.

If adding a dependency would point from a lower neutral crate into product or
composition, stop and run `cargo test -p ironclaw_architecture_tests` before proceeding.

Subagent spawn creates and wires child runs only. Planning, execution,
capability calls, checkpointing, gates, retries, and completion must continue
through the existing Reborn runner/driver/executor path.

Host-trusted trigger ingress is sealed by trigger-worker-owned request minting
and private conversation-owned trusted construction. Product adapters, product
workflow, first-party capabilities, and host-runtime handlers use untrusted
inbound requests and must not mint `TrustedInboundTurnRequest` or call trusted
trigger submitter factories.

## Coding and contract rules

- Do not use `.unwrap()` or `.expect()` in production. They are allowed in
  tests; production code propagates an explicit error.
- Keep clippy clean with zero warnings.
- Prefer `crate::` imports for cross-module references.
- Use strong types and enums for known domain shapes; raw strings belong at
  external boundaries.
- Shared types live with the contract owner. Do not create mirror DTOs or use
  `ironclaw_common` as a general dumping ground.
- Preserve existing defaults unless the task explicitly changes them.

## Persistence and configuration

New persistence behavior uses `RootFilesystem`/`ScopedFilesystem` and the mount
catalog owned by `ironclaw_filesystem`. Domain stores are thin typed wrappers
over scoped mounts and do not select or dispatch storage backends themselves.
Read-modify-write operations use the shared bounded CAS helper rather than a
process-local mutex held across backend I/O.

Keep bootstrap configuration, persisted settings, and encrypted secrets as
separate layers. Preserve configuration precedence, secret-mediated provider
resolution, and fail-closed startup behavior. Update the owning Reborn config,
product, composition, and frontend contracts when onboarding changes.

## Security and runtime invariants

- Treat every listener, route, product adapter, runtime lane, container, and
  external service as untrusted until a typed boundary establishes otherwise.
- Do not weaken authentication, origin checks, body limits, rate limits,
  allowlists, approval leases, secret mediation, or redaction guarantees.
- External HTTP goes through `ironclaw_network`; credentials remain host-side
  and are injected only through mediated runtime services.
- New ingress must validate and bound the original payload before persistence,
  prompt construction, credential injection, or runtime dispatch.
- Authorization, approval, reservation, dispatch, and execution are distinct
  stages. Do not bypass or collapse them.
- Session, thread, turn, and run identities are typed and must not be re-derived
  from display strings or transport metadata.
- Persistent memory is a filesystem-backed domain with document, chunking,
  indexing, search, and write-safety contracts—not transcript storage.

## Capabilities, extensions, and lifecycle

- Core host behavior uses typed built-in capabilities behind the same mediated
  host surface as other execution.
- Sandboxed extension execution belongs in WASM or a runtime lane; external
  server integrations belong behind MCP and the network boundary.
- Discovery is side-effect-free. Installation, credential binding, activation,
  execution, deactivation, and removal are explicit lifecycle transitions.
- Capability failures the model or user can correct are model-visible outcomes;
  host errors are reserved for failures that make the run unable to continue.
- Side-effecting success requires durable or provider-issued evidence plus
  read-back verification. If read-back is impossible, report the result as
  explicitly unverified rather than completed.

## Documentation and testing

`docs/` is the public Mintlify site plus fenced internal material. All new
internal engineering docs (design notes, research, plans, QA maps) go under
`docs/internal/` — nowhere else under `docs/`. A page outside the
`docs/.mintignore` fence is published even when omitted from `docs.json`
navigation (hidden pages stay reachable by URL), and `.mintignore` is frozen:
do not add entries. Enforced by `scripts/ci/docs_publication_boundary.py`
(Code Style workflow); run it to check placement.

Update the owning contract/docs when behavior changes. Choose tests using
`.claude/rules/testing.md` and
`.claude/skills/ironclaw-reborn-testing/SKILL.md`; use the smallest tier that
reaches the changed contract. Never commit secrets or PII. Before committing,
follow `.claude/rules/testing.md` and the owning crate guidance for clippy and safety checks.

Agents opening or updating a pull request must preserve and complete the
`Test Strategy` section from `.github/pull_request_template.md`. Every test tier
must include evidence or `Not applicable: <reason>`; do not present the pull
request as ready while fields are blank. Select tiers using
`docs/internal/testing-playbook.md` rather than running every tier by default.

## Change discipline

- Keep changes scoped and preserve unrelated work in dirty worktrees.
- Security, persistence schema, runtime, worker, CI, and secrets changes require
  explicit rollback, compatibility, and hidden-side-effect review.
- Avoid unrelated churn and generated-file edits.
- Run the narrowest meaningful checks, plus architecture tests when dependency
  ownership changes.
- Before finishing, re-check docs/parity requirements, security-sensitive paths,
  the final diff, and the exact files staged for commit.

## Before finishing

- Search changed production files for `.unwrap()`, `.expect()`, suspicious byte
  slicing, hardcoded temporary paths, and lost error causes.
- When a trait changes, enumerate all implementations, decorators, adapters,
  and test doubles; test through the production wrapper chain.
- When a pattern bug is fixed, search all `crates/` for sibling instances.
- For multi-step persistence, verify bounded CAS or a backend transaction and
  test interruption/concurrency behavior.
- For ingress, capabilities, events, and transports, verify actor scope, limits,
  redaction, and authoritative side-effect evidence.
- After moves/renames, search agent guidance, contracts, docs, tests, scripts,
  manifests, and frontend imports for old paths.
- Confirm the PR title/body describe every layer in the diff and explicitly note
  compatibility, rollback, and follow-up risks.
