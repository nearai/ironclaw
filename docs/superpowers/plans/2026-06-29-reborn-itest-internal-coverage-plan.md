# Reborn Integration-Test — Internal-Service Coverage Plan

**Date:** 2026-06-29
**Branch:** `reborn-itest-coverage` (off `107ffd7bc`, tip of framework PR #5392)
**Scope:** Use the now-landed in-process framework (#5392) to cover the **internal-service experiences** still defaulted/stubbed — approval gates + settings, auth/credential failure paths (revoke / 401 / reauth), and the other RootFilesystem-backed subsystems. This is the "later, overlap-minimized coverage effort" the design spec §1 deferred; NOT new framework machinery (slices 3+7 built that).

**Philosophy (unchanged):** real internal stack + real stores; mock only at the edges (scripted model at SDK seam, scripted egress for 401s). Tests terse (3–12 lines), zero-setup `cargo test`. Below the gateway — no HTTP/browser.

---

## Organizing lens: the product-adapter API surface

The coverage target is **every adapter-facing product API** — the doors a channel/gateway adapter calls. Confirmed: they all funnel through **one** door, `ProductWorkflow::submit_inbound` (`ironclaw_product_adapters/src/workflow.rs:38`), whose own contract doc lists the payload classes: *"create messages, runs, command/gate/auth outcomes, mission work, typed control actions."* The integration harness sits directly below that door, so coverage = "drive each payload class through `submit_inbound` (the harness's existing `submit_turn` already proves the create-message payload) and assert the real downstream store mutation / gate state."

This lens is why the slices below are achievable with the landed framework and why they don't need new machinery: each is a new **payload shape** through a proven door, routed to the matching service:

| Adapter API (payload class) | submit_inbound routes to | Status |
| --- | --- | --- |
| Create message → run | turn coordinator | ✅ covered (#5392 tool-call turn) |
| OAuth connect (grant gate) | `auth_interaction::resume_auth_gate` | ✅ covered (#5392) |
| OAuth refresh (background sweep) | refresh worker | ✅ covered (#5392) |
| **Approve gate** | `approval_interaction::approve_gate` | **C1** |
| **Deny gate** | `approval_interaction::deny_gate`/`resume_denied` | **C1** |
| **Change auto-approve setting** | `AutoApproveSettingStore::set` (CAS) | **C1** |
| **Add credential / auth token** | `auth_interaction::complete_selected_credential` + credential store | **C2** |
| **Revoke → 401 → reauth (+ deny)** | egress 401 → reauth gate; `resume_denied_auth` | **C2** |
| Cancel run | `submit_inbound` typed control payload | **C1 (one extra payload shape)** |
| Memory / projects / secrets / extensions / skills | in-turn tool dispatch (`memory_write`, `project_create`, …) | **C3/C4** |

C3/C4 are the *same* lens: those subsystems are driven as in-turn tool calls inside a `submit_inbound` create-message turn, then asserted at the store. So the entire plan is "adapter-API coverage," sliced by the service the payload reaches.

---

## Verified seams (from exploration — file:line confirmed)

**Approval:** real stores all exist with `Filesystem*`+`InMemory*` impls — `ApprovalRequestStore` (`ironclaw_run_state/src/lib.rs:216`), `CapabilityLeaseStore` (`ironclaw_authorization/src/lib.rs:235`), `AutoApproveSettingStore` (`ironclaw_approvals/src/auto_approve.rs:77`, default enabled). Gate raised = `RuntimeCapabilityOutcome::ApprovalRequired(RuntimeApprovalGate)` (`ironclaw_host_runtime/src/lib.rs:602,548`) → persisted `Pending` → `TurnStatus::BlockedApproval`. Resolve = `ApprovalResolver::approve_dispatch`/`deny` (`ironclaw_approvals/src/lib.rs:54,69`) then `runtime.resume_capability(...)`. `AutoApproveSettingStore::set(...)` is a **real CAS-persisted mutation** → "change setting → assert" is a real product test. Harness already has `disable_global_auto_approve_for_product_and_harness_users()` (`harness.rs:1869`) + `approve_local_dev_gate(gate_ref)` (`harness.rs:2375`); **no `deny_local_dev_gate` yet**. The `new_with_options`/local-dev harness path wires `approval_parts=Some`; the `core_builtin_tools` path has `approval_parts=None`.

**Auth failure:** revoke = `CredentialAccountService::update_status(.., Revoked)` (`ironclaw_auth/src/credential.rs:487`; no delete method — `Revoked` is terminal). `invalid_grant` from the token endpoint auto-sets `Revoked` in `refresh_account` (`credential.rs:969`). 401 reaction = `enrich_dispatch_error_credential_requirements` (`ironclaw_capabilities/src/host.rs:2285`) → `DispatchError::AuthRequired` → reauth gate; **reactive-refresh-on-401-retry does NOT exist (pre-existing gap, documented `host.rs:2276-2283`)** — the gate is the correct behavior for a genuinely-revoked token. Reauth resolve = `GateResumeDisposition::{Granted,Denied}` via `AuthInteractionService::resume_auth_gate` (`ironclaw_product_workflow/src/auth_interaction/service.rs:163`). **`ScriptedOAuthTokenEgress` (`test_support.rs:307`) + `RecordingRuntimeHttpEgress` (`harness.rs:3471`) hardcode status 200**; `ScriptedHttpResponse` (`http_matcher.rs:32`) has **no status field** → scripting a 401 needs a small `status: u16` addition (test-support only).

**Inventory (RootFilesystem-backed, ranked):** EASY = Memory (`ironclaw_memory_native/src/repo/filesystem.rs:83`), Projects (`ironclaw_projects/src/store.rs:47`), Secrets-`FilesystemSecretStore` (`ironclaw_secrets/src/filesystem_store.rs:198`), Profile (sub-case of Memory, `user_profile_source.rs:82`). MEDIUM (need a harness accessor) = Extensions (`extension_installation_store.rs:76`), Skills (`filesystem_skill_bundle_source.rs:107`), Conversation metadata (`ironclaw_conversations/src/filesystem_store.rs:77`). HARD/OUT = Triggers (own SQL DB, **not** RootFilesystem), Capability leases (InMemory-only, ephemeral by design — covered behaviorally by the approval slice).

---

## Slices (prioritized; user priority = approval/auth first)

### C1 — Approval-gate lifecycle + settings (priority 1) — `tests/reborn_integration_approval_gates.rs`
Test, in-process through the integration harness: scripted destructive tool call → (auto-approve disabled) real gate raised → **approve** → resume → completes; → **deny** → model sees a denial, not a hang; → **setting flip** (ask-every-time vs always-approve) asserts gate-vs-no-gate on the *same* capability.
- **BLOCKING primitive — generalize, don't fuse (round-1 review: flagged by thermo + approach + maintainability):** `submit_turn` (`builder.rs:483`) polls the private `wait_for_completion` (`builder.rs:647`) until `Completed`/terminal, but `TurnStatus::BlockedApproval` is **not** terminal (`ironclaw_turns/src/status.rs:34`) — so an approval-gated turn times out and the test can never approve. Do **NOT** add a parallel `submit_turn_until_blocked` poll loop. Instead **generalize the existing loop** into `wait_for_status(run_id, expected: TurnStatus) -> HarnessResult<TurnRunState>` (stop on `expected` OR `is_terminal()`), mirroring the canonical one already on the binary-E2E harness (`harness.rs:1201`, `wait_for_status_in_scope_with_config` at `:1239`). Then `wait_for_completion`'s body becomes `wait_for_status(run_id, Completed)`; the approval test calls `wait_for_status(run_id, BlockedApproval)`; **C2's reauth test reuses the same primitive with `BlockedAuth`** (`status.rs:18`) — one loop, three callers, zero duplication. `submit_turn_until_blocked(text)` is then a thin 2-line convenience wrapper = `submit_turn_async + wait_for_status(BlockedApproval)` returning `(TurnRunId, GateRef)` (kept as the named fixture for C1's terse test bodies). Test flow = `submit_turn_until_blocked → approve/deny gate → wait_for_status(Completed)`.
- **Module placement — keep impl where the fields live (round-1 review, thermo):** the existing `approve_local_dev_gate` (`harness.rs:2375`) reads six **private** `HostRuntimeCapabilityHarness` fields (`approval_parts`, `pending_approval_scopes`, `capability_mount_overrides`, `effect_kinds`, `mounts`, `network_policy`). Moving its body to a free function in `approval.rs` would force those fields to `pub(super)` (visibility leak) or become a circular delegation stub — neither is cleaner. Decision: **the gate-resolution implementation stays on `HostRuntimeCapabilityHarness` in `harness.rs`**; add the new `deny_local_dev_gate` **beside** `approve_local_dev_gate` there (same fields, cohesive, ~20 lines on an already-arch-exempt file). Expose the consumer-facing **`approve_gate(gate_ref)` / `deny_gate(gate_ref)`** as thin methods on `RebornIntegrationHarness` in **`builder.rs`** (next to `submit_turn`), delegating to `self.capability_mode.{approve,deny}_local_dev_gate`. `approval.rs` holds **types only** (`ApprovalWaitConfig` + any `GateRef` re-export) — no logic. This adds the consumer API in one place, keeps the impl with its data, and introduces zero field-visibility changes.
- **Naming (round-1 review, local-patterns):** the builder's opt-in family is mixed — `with_builtin_http_tools`, `with_mock_mcp`, `with_live_shell`, `with_keyed_http_responses` — i.e. `with_<disposition>_<subsystem>`, not uniformly `with_builtin_*`. The approval opt-in swaps the default stubbed-out gating (auto-approve, no gate) for the real gate path, so it matches the **`with_live_*`** disposition (`with_live_shell` swaps recording-stub→real). Use **`.with_live_approvals()`** (NOT `with_approval_gates`/`with_builtin_approval_gates`/`with_real_approvals`) so the chosen name and the documented idiom agree: `with_builtin_*` = enable a built-in tool surface; `with_live_*` = swap a recording/no-op stub for the real subsystem; `with_mock_*` = inject a scripted fake. Reuse `disable_global_auto_approve_for_product_and_harness_users` + `AutoApproveSettingStore::set` (real CAS-persisted mutation).
- **Overlap discipline — corrected tier (review finding):** the 1263-line `approval_gates.rs` is at `crates/ironclaw_reborn_composition/src/factory/local_dev_host_tests/approval_gates.rs` — the **HostRuntime/composition tier** (raw `invoke_capability`/`resume_capability`, NO scripted model, NO turn loop). `reborn_approval_traces_parity.rs` is the **binary-E2E tier** (covers approve+cancel, not deny-disposition). C1 owns ONLY the genuinely-uncovered **scripted-SDK turn-loop-from-the-model's-POV** path (model emits tool call → gate in `TurnStatus` → resolve → model sees the outcome in its next reply). It MUST NOT re-test gate *mechanics* (lease types, policy/permission overrides) that `approval_gates.rs` already owns.
- **Cancel-run (adapter-API lens):** while a turn is `BlockedApproval`, a typed *control* payload through `submit_inbound` cancels the run. This is one extra payload shape on the **same** `submit_turn_until_blocked` fixture (block → cancel instead of approve → assert terminal `Cancelled`, not a hang). Add it **only if** it reuses that fixture with no new harness machinery; otherwise defer to a follow-up (do not invent a cancel seam for it).

### C2 — Auth/credential failure paths (priority 1) — `tests/reborn_integration_auth_failure.rs`
- **Two distinct change surfaces (review finding):** (i) `status: u16` (default 200) on `ScriptedHttpResponse` (`tests/support/reborn/http_matcher.rs`) — pure test-tree change, matches the existing `with_method`/`with_capability` builder shape; (ii) `status`/error path on `ScriptedOAuthTokenEgress` — this lives in `crates/ironclaw_reborn_composition/src/test_support.rs` (a **gated crate-API change** behind the `test-support` feature, backward-compatible/additive — NOT "test-only"; frame it accurately in the PR).
- **Negative guard for C2(c) (review finding):** the 401→reauth-gate test must prove the **401 is the trigger** — run the same flow with the egress returning **200** and assert **no** reauth gate (else a `BlockedAuth` for an unrelated reason, e.g. no credential configured, would pass vacuously).
- Tests: (a) **revoke** → `update_status(Revoked)` → read back `Revoked`; (b) **invalid_grant** end-to-end → refresh sweep hits a scripted `invalid_grant` token response → account auto-marked `Revoked` (reuses slice-8 `sweep_for_refresh`); (c) **runtime 401** from a tool egress → `enrich_dispatch_error_credential_requirements` surfaces a **reauth gate** (assert the gate, NOT a refresh-retry — that retry doesn't exist); (d) reauth **deny** → `GateResumeDisposition::Denied` resolution; (e) **add credential** (positive baseline) → `complete_selected_credential` persists a usable account → a subsequent same-capability tool egress runs **without** a gate. (e) is the constructive complement to (a)/(c) and reuses the same `OAuthProductAuthTestBundle`; it doubles as the negative guard's positive arm.
- Reuse the slice-7/8 `OAuthProductAuthTestBundle`; extend, don't fork.

### C3 — EASY RootFilesystem batch (priority 2) — `tests/reborn_integration_{memory,projects,secrets}.rs`
One small test each, reusing the persist+reopen *pattern* (not the file) from `backend_matrix`. One file per subsystem (memory+profile share `reborn_integration_memory.rs` since profile is a memory sub-case):
- **Memory**: scripted `memory_write`→`memory_search`/`read`/`tree`, assert **membership** (FTS — embeddings never consulted, per slice-9); under LibSql, reopen + assert readback. **Profile**: `profile_set` → reopen → `resolve_user_profile` (sub-case, same file).
- **Projects**: scripted `project_create` → reopen (LibSql) → assert record present.
- **Secrets**: wire `FilesystemSecretStore` under `StorageMode::LibSql` (a one-liner branch; §3.8 exception #2) → write credential → reopen → readback.

### C4 — MEDIUM (priority 3; each needs a harness accessor)
- **Extensions** install→activate→remove → `/system/extensions/.installations/state.json` persisted → reopen survives.
- **Skills** install → second turn sees the skill prompt injected (needs `FilesystemSkillBundleSource` wired; harness sets `skill_context_source: None` today).
- **Conversation metadata** (thread id/timestamps/session boundaries) → its **own** `reborn_integration_conversation.rs`. (Review finding: do NOT extend `backend_matrix` — CLAUDE.md scopes it to thread+turn only; conversation-state is a distinct subsystem.)

### Deferred / out of scope (with reasons)
- **Triggers/routines** — persist to a separate SQL DB, **not** RootFilesystem; needs a different harness seam (trigger repo accessor) → its own later effort, not "mostly wiring."
- **Capability leases** — InMemory-only by design (ephemeral); behaviorally covered by C1. No persistence gap to close.

---

## Cross-cutting decisions to flag for review
1. **Real-approval path on the integration harness:** the scripted-SDK harness defaults to `core_builtin_tools` (`approval_parts=None`). C1 needs the real-approval wiring. Decision: add a **`.with_live_approvals()`** builder opt-in (naming per the C1 finding — `with_live_*` = swap a no-op stub for the real subsystem, matching `with_live_shell`) routing to the local-dev `approval_parts=Some` path, vs. always wiring approval stores. Prefer opt-in — keeps the default terse + fast.
2. **`status` field on the scripted egresses:** small test-support type change (ScriptedHttpResponse + ScriptedOAuthTokenEgress). Default 200, additive — no production change.
3. **PR grouping:** propose 3 PRs — C1 (approval), C2 (auth-failure), C3 (easy batch); C4 as a follow-up. Each rebases on the merged framework PR #5392. Keeps review surfaces small.
4. **Overlap discipline (the hard one):** crate-level tests already cover approval (1263 lines) + auth contracts; `reborn_approval_traces_parity` covers the binary-E2E tier. These new slices must cover the **scripted-SDK integration tier specifically** and NOT re-assert what those already prove — extend/consolidate, add only the genuinely-distinct scenario, and say why each new test exists.

---

## Verification (each slice)
Test-first (write the failing test, confirm it fails for the right reason); terse bodies; run the **exact** target — `cargo test --test reborn_integration_approval_gates` (C1) / `--test reborn_integration_auth_failure` (C2) / `--test reborn_integration_{memory,projects,secrets}` (C3) — zero setup, default features. **Guard against false-green:** `cargo test --test <name>` against a *non-existent* target exits 0 (no-op), so after creating each file confirm the target actually ran ≥1 test (non-zero test count in output), not just exit 0. Then `cargo fmt --check`; full CI clippy `cargo clippy --all --tests --examples --all-features -- -D warnings`; the no-panics check; for any test asserting a *reaction* (gate raised, status persisted) a negative guard proving it isn't vacuous (the mutation-testing lesson from #5392); and **update `tests/support/reborn/CLAUDE.md` "Implemented now vs planned"** as each slice lands (review finding — established discipline).

> **Line-number corrections (review):** in "Verified seams" — `update_status` is `credential.rs:503` (not :487); `MemoryBackedUserProfileSource` is `user_profile_source.rs:51`; `ExtensionInstallationStore` struct is `extension_installation_store.rs:14`. Substance unchanged; fix anchors before implementing. "§3.8 exception #2" = the design-spec note that some capability-harness variants use `StaticSecretStore` (no-op writes), so a real secret write+read-back needs the `FilesystemSecretStore` path.
