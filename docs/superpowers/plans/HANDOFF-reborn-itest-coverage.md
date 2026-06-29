# Handoff — Reborn integration-test internal-coverage effort

## Where you are
- **Worktree:** `/Users/henry/Code/ironclaw-wt/reborn-itest-coverage`
- **Branch:** `reborn-itest-coverage` (base `107ffd7bc` = tip of merged-pending framework PR #5392)
- **Plan (REVIEW-CLEAN, implement this):** `docs/superpowers/plans/2026-06-29-reborn-itest-internal-coverage-plan.md`
- **This handoff:** `docs/superpowers/plans/HANDOFF-reborn-itest-coverage.md`
- Both docs are currently **untracked** (uncommitted). Commit when you start.

## Status
Plan is review-clean: 2 rounds of thermo-nuclear + 3 code-review reviewers (approach/local-patterns/maintainability), all APPROVE/`[]` on round 2. **No flagged items remain.** Code is NOT yet written.

## What this is
Use the already-landed #5392 in-process Reborn integration-test framework to cover the **product-adapter API surface** still uncovered. Philosophy unchanged: run the real internal stack, mock only at the bottom edges (scripted model at the SDK seam; scripted egress for 401s). Below the gateway — no HTTP/browser. Tests terse (3–12 lines), zero-setup `cargo test`.

**Organizing lens:** every adapter door funnels through ONE method `ProductWorkflow::submit_inbound` (`crates/ironclaw_product_adapters/src/workflow.rs:38`). Each slice = a new payload SHAPE through that proven door + assert the downstream store/gate. Covered today (#5392): create-msg turn, oauth connect, oauth refresh. Gap = C1/C2/C3/C4 below.

## Slices (priority order)
- **C1 — approval gates + settings** (priority 1) → `tests/reborn_integration_approval_gates.rs`
- **C2 — auth/credential failure** (priority 1) → `tests/reborn_integration_auth_failure.rs`
- **C3 — EASY RootFilesystem batch** (priority 2) → `tests/reborn_integration_{memory,projects,secrets}.rs` (memory+profile share one file)
- **C4 — MEDIUM** (extensions/skills/conversation) → follow-up, own files
- **Deferred:** triggers (own SQL DB, not RootFilesystem), capability leases (InMemory-only, covered behaviorally by C1)
- **PRs:** C1, C2, C3 each off #5392; C4 follow-up.

## Converged decisions (HONOR THESE — they are the review outcome)
1. **Wait primitive — GENERALIZE, don't fuse.** Do NOT add a parallel poll loop. Generalize `builder.rs:647` `wait_for_completion` → `wait_for_status(run_id, expected: TurnStatus) -> HarnessResult<TurnRunState>` (stop on `expected` OR `is_terminal()`), mirroring the canonical one already on the binary-E2E harness (`tests/support/reborn/harness.rs:1201` + `:1239`). `submit_turn` calls it with `Completed`; C1 approval test with `BlockedApproval`; C2 reauth test with `BlockedAuth` (`ironclaw_turns/src/status.rs:18`). ONE loop, three callers. `submit_turn_until_blocked(text)` = thin 2-line wrapper (`submit_turn_async + wait_for_status(BlockedApproval)`) returning `(TurnRunId, GateRef)`, kept as C1's named fixture.
2. **Gate impl stays where its data lives.** `approve_local_dev_gate` (`harness.rs:2375`) reads 6 PRIVATE `HostRuntimeCapabilityHarness` fields — do NOT relocate to `approval.rs` (would force `pub(super)` leak or circular stub). Impl + new `deny_local_dev_gate` stay on `HostRuntimeCapabilityHarness` in `harness.rs` (~20 lines; file is arch-exempt). Consumer-facing `approve_gate(gate_ref)`/`deny_gate(gate_ref)` = thin methods on `RebornIntegrationHarness` in `builder.rs` (next to `submit_turn`), delegating via `self.capability_mode`. `approval.rs` = TYPES ONLY (`ApprovalWaitConfig` + `GateRef` re-export), no logic.
3. **Naming:** opt-in is `.with_live_approvals()` (NOT `with_approval_gates`/`with_real_approvals`). Builder family is mixed: `with_builtin_*` = enable a built-in tool surface; `with_live_*` = swap a no-op/recording stub for the real subsystem (sibling of `with_live_shell`, `builder.rs:176`); `with_mock_*` = scripted fake. Reuse `disable_global_auto_approve_for_product_and_harness_users` + `AutoApproveSettingStore::set` (real CAS-persisted mutation) for the settings-flip test.
4. **C2 egress status field — two distinct surfaces.** (i) `status: u16` (default 200) on `ScriptedHttpResponse` (`tests/support/reborn/http_matcher.rs`) = pure test-tree change. (ii) `status`/error path on `ScriptedOAuthTokenEgress` (`crates/ironclaw_reborn_composition/src/test_support.rs`, behind `test-support` feature) = gated crate-API change, additive/backward-compatible — frame accurately in the PR (NOT "test-only").
5. **Tier discipline (overlap).** C1/C2 own ONLY the scripted-SDK turn-loop-from-the-model's-POV path. Do NOT re-test gate mechanics already owned by the HostRuntime tier (`crates/ironclaw_reborn_composition/src/factory/local_dev_host_tests/approval_gates.rs`, 1263 lines) or the binary-E2E tier (`tests/reborn_approval_traces_parity.rs`).

## Verification discipline (per slice)
- **Test-first:** write the failing test, confirm it fails for the RIGHT reason, then implement. Red → green.
- Run the EXACT target: `cargo test --test reborn_integration_approval_gates` (C1) / `--test reborn_integration_auth_failure` (C2) / `--test reborn_integration_{memory,projects,secrets}` (C3). Default features, zero setup.
- **False-green guard:** `cargo test --test <name>` on a NON-EXISTENT target exits 0 (no-op). After creating each file confirm a non-zero test count actually ran — not just exit 0.
- `cargo fmt --check`; full CI clippy `cargo clippy --all --tests --examples --all-features -- -D warnings`; the no-panics check (`scripts/check_no_panics.py`).
- **Negative guard per reaction-asserting test** (the #5392 mutation-testing lesson): any test asserting a *reaction* (gate raised, status persisted) needs a control proving it isn't vacuous. Mutation-test it: break the code, confirm the test goes RED.
- **Update `tests/support/reborn/CLAUDE.md`** "Implemented now vs planned" as each slice lands.

## Gotchas (from the #5392 build)
- Subagent narration is UNRELIABLE — always re-run tests yourself to verify counts/results.
- Full all-features clippy catches `type_complexity`/`derivable_impls`/`private_interfaces` that targeted `-p` builds miss — run it before any PR.
- New shared support modules need module-level `#![allow(dead_code)]` (all-features lane flags cross-binary dead code).
- "Code Style (fmt+clippy)" CI job is an AGGREGATOR — if it fails, check the named sub-result (often no-panics), not fmt/clippy.
- `libsql` is a default feature; plain `cargo test --test 'reborn_*'` runs zero-setup.

## Read first in the new session
- The plan: `docs/superpowers/plans/2026-06-29-reborn-itest-internal-coverage-plan.md`
- Authoring guide / tier defs: `tests/support/reborn/CLAUDE.md`
- Support files: `tests/support/reborn/{builder.rs,harness.rs,approval.rs,http_matcher.rs}`
- codegraph IS initialized in this worktree — use `codegraph_context`/`codegraph_explore` over grep/read loops.
- Auto-memory (loaded each session): `project_reborn_itest_coverage` (this effort) + `project_reborn_itest_slices_3_9` (the #5392 framework it builds on).

## First action
Implement C1 (approval gates), test-first, per the converged decisions above. Start by generalizing `wait_for_completion` → `wait_for_status` in `builder.rs`, then `.with_live_approvals()` opt-in, then `deny_local_dev_gate` + `approve_gate`/`deny_gate` wrappers, then the test file.

## Post-implementation cleanup (MANDATORY final step — do NOT skip)
Slice numbers and plan/spec pointers are scaffolding for building the framework. **Once implemented they are stale and misleading** — a reader should learn what the support modules DO, not the historical order they were added. After the last slice lands (and before/with the final PR):

1. **Rewrite `tests/support/reborn/CLAUDE.md` to be slice-agnostic.** Remove ALL "Slice N ships …" prose (currently ~lines 75–204) and the entire **"Implemented now vs planned"** section (line 105) including the **"Planned"** list (~line 210). Replace with a capability-organized description: each support module / builder opt-in / assertion documented by *what it provides and how to use it*, present-tense, no slice numbers, no "planned vs implemented" split. If something is genuinely not built yet, it simply isn't in the doc (no dead-code, no speculative entries).
2. **Drop stale plan/spec pointers** from CLAUDE.md: the `docs/superpowers/specs/2026-06-26-…-design.md` reference (line 103) and any plan-doc links — the design spec was the build-time tiebreaker; the shipped tests are now the source of truth. Keep a one-line "design rationale: see git history / the design spec" pointer at most, not inline spec-section anchors (`§3.6`, `§3.8`, `§9 step 8`, etc.) scattered through the prose.
3. **Delete the now-consumed planning docs** once their content is fully reflected in code + CLAUDE.md: `docs/superpowers/plans/2026-06-29-reborn-itest-internal-coverage-plan.md` and this `HANDOFF-reborn-itest-coverage.md`. They are working scaffolding, not durable docs.
4. Sanity check: `grep -riE "slice|planned|§[0-9]" tests/support/reborn/CLAUDE.md` should return nothing meaningful afterward.

Until then, the per-slice "update CLAUDE.md 'Implemented now vs planned'" bullet in **Verification** is fine as *interim* tracking — it gets erased by this final cleanup.
