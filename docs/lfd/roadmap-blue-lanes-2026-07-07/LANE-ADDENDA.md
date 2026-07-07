# LANE-ADDENDA — per-lane corrections, brief bindings, waves

Applies on top of each lane's `goal.md` + `../COMMON.md` + `INSTRUMENTS.md`.
"Brief" = the designer loss-function brief in `lfd/_briefs/` that seeds the
lane's eval package. Stage-0 commands are additive to the repo-wide gate
(`cargo fmt` + `cargo clippy --all --benches --tests --examples
--all-features` with zero warnings).

## Launch waves (assign one agent per lane, one worktree per agent)

| Wave | Lanes | Rationale |
| --- | --- | --- |
| 1 | 03 slack · 04 secrets · 06 cleanup · 09 custom-build-tools · 10 voice · 11 missions · 16 memory-placement | disjoint primary surfaces |
| 2 | 05 onboarding (needs 03) · 07 admin-config · 17 write-pipeline + 18 retrieval (need 16) · 19 benchmarks (parallel to 17/18 — its calibration uses synthetic known-good/known-bad variants, not the real pillars) | |
| 3 | 08 permission-NL (after 07 — shared authorization surface) · 12 collab (after 07/08) · 13 self-learning umbrella (after 17/18/19) · 02 must-have bundle (after 03/05) | |
| 4 | 15 memory-platform meta (after 16–19) · 14 long-term-memory acceptance (after 15) · 01 reborn umbrella rollup (last / continuous) | umbrellas score pillars, they don't re-implement them |

Surface-conflict table (never run two lanes from one cell concurrently):
`ironclaw_reborn_composition` slack modules {03, 05} ·
host_runtime credential mediation {04, 09} ·
authorization/approvals/capabilities {07, 08, 12} ·
memory family crates {13, 14, 15, 16, 17, 18} ·
`FEATURE_PARITY.md` and parity docs {01, 06, every lane at closeout — serialize doc edits}.

## Lane 01 — Reborn umbrella

- **Recast** (per REVIEW finding 5): not an independent code-editing loop.
  Its scorer is a meta-rollup: weighted read of the pillar lanes' holdout
  aggregates + its own 15–20 cross-cutting scenarios (auth turn, routine,
  connector route, secret mediation, operator recovery). Runs last or as a
  standing dashboard.
- Path fix: `crates/ironclaw_webui_v2_static` → `crates/ironclaw_webui_v2/static/`
  + `crates/ironclaw_webui_v2/frontend/`.
- Brief: designer-authored at package time (no `lfd/_briefs/` entry yet);
  seeds: `scripts/reborn_qa_matrix`, `tests/fixtures/llm_traces/reborn_qa/`.
- Stage 0: `cargo test --features integration --test integration` (full
  Reborn integration target) + webui-v2 JS tests.

## Lane 02 — NEAR Foundation must-haves

- Schedule after 03/05 (it consumes their surfaces); treat as a
  bundle-verification loop with small code deltas, not feature-building.
- 10 dev journeys is thin for 6 must-haves — designer grows toward 2 dev +
  3 holdout journeys per must-have before launch.
- Brief: designer-authored at package time; seeds: lane 03/05 packages,
  `tests/e2e/` Emulate fixtures (Google/Slack/GitHub), MCP factory tests.
- Stage 0: e2e smoke subset + `cargo test --features integration` groups
  touched by the bundle.

## Lane 03 — Slack as main channel

- Brief: `lfd/_briefs/slack-channel.md`. The lane's Model-1 scope
  (channel↔agent mapping, no user-token posting) SUPERSEDES the brief's
  generic parity framing; brief's eval themes and Slack-id caps carry over.
- Surface addendum: the composition seam is richer than the goal implies —
  `crates/ironclaw_reborn_composition/src/slack_{actor_identity,channel_connection,channel_routes,delivery,egress,personal_binding*,serve,setup}.rs`.
- Stage 0: `ironclaw_slack_v2_adapter` crate tests +
  `cargo test --features integration --test integration slack` +
  `tests/reborn_group_triggers/` delivery cases (triggered Slack delivery
  has recent seams, PR #5719/#5735).
- Wave 1.

## Lane 04 — Secrets usage with Skills/Tools

- Brief: `lfd/_briefs/secrets-skills-tools.md` (bar there 0.95 vs lane
  0.94 — keep 0.95, INSTRUMENTS lets lanes tighten only).
- Cited seams all verified: `crates/ironclaw_host_runtime/src/egress/credential.rs`,
  `obligations.rs`, `tests/skill_credential_injection.rs`,
  `tests/integration/secret_injection.rs`. Extend those tests, don't fork.
- Stage 0: the two cited test files + `ironclaw_secrets`/`ironclaw_authorization`
  crate tests + lease-expiry parking coverage (PR #5723 seam).
- Wave 1.

## Lane 05 — Channel-first onboarding

- Brief: `lfd/_briefs/onboarding-channel-first.md`. Keep the brief's
  synthetic-registry-channel holdout fence (unseen channel must route via
  the generic resolver) — it's the strongest anti-hardcode fence and the
  lane lacks it.
- Path fix: `crates/ironclaw_webui_v2_static` → `crates/ironclaw_webui_v2/{static,frontend}/`.
- Preserve the repo invariant: route by `extension_name`, never
  `credential_name` (root CLAUDE.md); holdout includes the shared-credential
  disambiguation case from the brief.
- Stage 0: `ironclaw_extensions` + `ironclaw_webui_v2` (Rust + JS) +
  `tests/reborn_group_extensions/`.
- Wave 2 (after 03).

## Lane 06 — Clean up old architecture

- Brief: `lfd/_briefs/cleanup-old-architecture.md`. **Scope delta to
  reconcile:** the brief scores migration fidelity (seeded v1 DBs →
  `ironclaw_reborn_migration` → projected Reborn state, deletion unscored);
  the lane scores deletion safety (ledger + reference checks). Combine:
  fidelity contracts from the brief become the lane's "replacement
  evidence" hard gate; the lane's deletion-ledger scoring stands. Both
  agree deletion never scores by LOC.
- Stage 0: `tests/migration_roundtrip.rs` (gap set may only shrink) +
  `ironclaw_reborn_migration` crate tests + a new `Command::Migrate` CLI test.
- Wave 1.

## Lane 07 — Admin configurable skills/tools

- Brief: `lfd/_briefs/admin-skills-tools.md`. Carry the brief's fences the
  lane lacks: synthetic tool names (probe-renamable), deny-by-default on
  empty store, paired mutation+audit matchers, `dispatch-exempt` additions
  capped at 0.
- Cited seam verified: `crates/ironclaw_product_workflow/src/reborn_services*`.
- Stage 0: `ironclaw_webui_v2` settings routes tests + `ironclaw_skills` +
  `ironclaw_authorization` crate tests.
- Wave 2.

## Lane 08 — Permission management (natural language)

- Brief: `lfd/_briefs/permission-management.md` covers the SUBSTRATE
  (gate surfacing, resume identity, deny-continue, leases, mid-gate
  refresh). The lane adds the NL command layer on top. Split Stage 0
  accordingly: substrate parity suites green (brief) BEFORE the NL layer
  descends (lane). The brief's eval package ships as the lane's
  "substrate" contract group; NL cases are a second designer-authored group.
- The lane's core rule matches repo Rule 5 exactly: model classifies,
  deterministic policy decides. Keep the 100/200 eval counts as designer
  growth targets (finding 3).
- Stage 0: `tests/reborn_group_approvals/` + `reborn_approval_traces_parity`
  + `budget_approval_e2e` + `ironclaw_run_state` crate tests.
- Wave 3 (after 07).

## Lane 09 — Custom build tools

- Brief: `lfd/_briefs/custom-build-tools.md`. **Scope delta:** the lane
  narrows to ONE tool shape (credentialed HTTP API wrapper) — accept the
  narrowing (good LFD practice); the brief's WASM compile/sandbox themes
  become the artifact/validation layer for that shape. Brief fences carry
  over: no committed binary blobs (cap 0), provenance state_pred,
  per-case IO transforms so echo-tools fail.
- Stage 0: `ironclaw_wasm*` + `ironclaw_extensions` crate tests; toolchain
  probe (`wasm32-wasip2`) per the brief if the artifact format needs it.
- Wave 1.

## Lane 10 — User-voice model

- Brief: `lfd/_briefs/user-voice-model.md` — **corrected**: an STT seam
  already exists at `crates/ironclaw_llm/src/transcription/` (OpenAI-shaped
  + chat_completions transcription); the lane surface allowlist is right.
  Build the provider abstraction there, not a new crate.
- Keep the brief's fixture protocol (macOS `say`-generated WAVs, sealed
  reference transcripts, content-hash-keyed mock table in pinned support,
  `transcript_wer` matcher — already implemented in the shared scorer).
- The lane's text-equivalence framing (voice routes identically to
  equivalent text) is a stronger target than the brief's — adopt it as a
  contract group.
- Stage 0: transcription module tests + attachments/extractors crate tests.
- Wave 1.

## Lane 11 — Missions

- Brief: `lfd/_briefs/missions.md`. **Scope delta:** the lane's
  state-machine framing (goal, definition-of-done, budget, checkpoints,
  routine-vs-task decisions) supersedes the brief's meta-prompt focus;
  brief themes that survive: meta-prompt-from-memory data-flow contracts,
  probe-varied adaptation (`next_focus`), no-duplicate-mission-identity,
  restart durability via group storage reload.
- Stage 0: `ironclaw_triggers` + `tests/reborn_group_triggers/` +
  `crates/ironclaw_turns` long-running profile tests.
- Wave 1.

## Lane 12 — Multi-tenant cross-agent collaboration

- Brief: `lfd/_briefs/multi-tenant-collab.md`. Adopt the brief's
  acceptance form: bar AND zero isolation violations on holdout (the lane
  has it too — keep both-sided pricing: denial-only implementations fail
  the required-collaboration themes).
- Surface note: `crates/ironclaw_product_context` exists (verified) — the
  context-crossing envelope work likely lands there + `ironclaw_loop_support`
  spawn paths.
- Stage 0: ALL `tests/reborn_*_scope_isolation_parity.rs` +
  `reborn_subagent_spawn_e2e` — the isolation floor; weakening any is VOID
  territory (caps: `#[ignore]` additions = 0).
- Wave 3 (after 07/08).

## Lane 13 — Self-learning loops (umbrella)

- Brief: `lfd/_briefs/self-learning-loops.md`. **Reconciliation:** the
  lane reframes learning around memory write/retrieval pillars; the
  brief centers skill distillation (`ironclaw_skill_learning`
  distill/refine + auto-fire). Keep BOTH: brief themes become the
  "skill-artifact" contract group inside the lane's eval; the lane's
  longitudinal two-session scenarios are the umbrella frame.
- Depends on 17/18/19 sub-scorers per its own harness design — wave 3.
- Prior-art pointer from the repo owner's notes: branch
  `claude/reborn-learning-system` (read for design context; build fresh).
- Stage 0: `ironclaw_skill_learning` + `ironclaw_skills` crate tests +
  pillar Stage-0 suites of 17/18.

## Lane 14 — Long-term memory (product acceptance)

- Brief: `lfd/_briefs/long-term-memory.md` maps mostly to lane 18's
  retrieval scope; lane 14 is the product-acceptance umbrella over 15–18
  (resident/long-term split, longitudinal repeated-question deltas).
  Recast like lane 01: meta-scorer over pillar holdouts + its own
  longitudinal scenario group. Wave 4 (after 15).
- The brief's never-delete/versioning retention contracts move into lane
  17 (write) and lane 16 (placement parity) packages so they aren't lost.

## Lane 15 — Reborn memory platform (meta)

- Path fix: `docs/reborn/2026-06-22-memory-as-product-layer.md` does not
  exist (tree or history). Real seeds:
  `docs/plans/2026-06-23-hermes-style-context-management.md`,
  `contracts/memory.md`, `contracts/memory-profiles.md`,
  `contracts/storage-placement.md`. Record the substitution in LOG.md as
  the lane already instructs.
- Meta-scorer over 16/17/18/19 exactly as written (this lane got the
  umbrella shape right — lane 01/14 are recast to match it). Wave 4.
- Brief: designer-authored at package time.

## Lane 16 — Memory placement (product/provider boundary)

- No corrections; cited crates verified. The fake-provider probe (product
  code must not special-case native memory) is the load-bearing fence —
  the designer package pins the fake provider in shared support, NOT in
  the lane profile (else the implementer could special-case around it).
- Brief: designer-authored at package time; retention/versioning
  contracts inherited from `lfd/_briefs/long-term-memory.md` (see lane 14).
- Stage 0: `ironclaw_memory` + `ironclaw_memory_native` crate tests +
  `tests/integration/group_memory/`.
- Wave 1.

## Lane 17 — Self-learning write pipeline

- Eval counts (120/300) are designer growth targets; initial designer set
  40 dev / 14 holdout across the 9 signal classes (finding 3).
- The closed artifact-type enum + "classifier output is not authority"
  rules are exactly Rule-5-shaped — keep verbatim.
- Brief: designer-authored at package time; inherits write-path +
  retention themes from `lfd/_briefs/long-term-memory.md` and the
  distillation-validity fences from `lfd/_briefs/self-learning-loops.md`.
- Stage 0: `ironclaw_memory*` crate tests + `group_memory` write cases.
- Wave 2 (after 16).

## Lane 18 — Long-term memory retrieval pipeline

- Direct descendant of `lfd/_briefs/long-term-memory.md` retrieval themes
  (per-doc required/forbidden matchers, envelope token-budget state_pred,
  synonym cases that force semantic — not lexical — match via the pinned
  deterministic embedding fake).
- Retrieval-count cap ("at most 8 unless spec and scorer change together")
  binds scorer-side: only the designer can change it.
- Stage 0: `group_memory` retrieval cases + `reborn_qa_doc_grounding`.
- Wave 2 (after 16).

## Lane 19 — Memory benchmarks and evaluation

- **Boundary with the shared harness:** lane 19 builds MEMORY benchmarks
  (product eval infra, reusable in CI) — it does not replace
  `lfd/_shared`. Its calibration gate (known-good passes, every known-bad
  fails for the intended reason) must itself run under the shared scorer
  as meta-contracts: the benchmark verdict per seeded variant is the
  outcome; sealed expected verdicts are the contract.
- Eval counts (220/560) are growth targets; initial designer set: 7 seeded
  variants (good, write-everything, write-nothing, retrieve-everything,
  retrieve-nothing, cross-scope-leak, stale-injection, no-attribution)
  × 5–6 scenario families ≈ 40 dev / 14 holdout meta-cases.
- Wave 2 (parallel to 17/18; its variants are synthetic).
- Stage 0: benchmark harness unit tests + one `reborn_qa` trace replay.
