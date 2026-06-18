# Reborn approval-gate denial: surface to model and continue (mirror #4944)

Status: PLAN (reviewed — approach/local-patterns/maintainability)
Date: 2026-06-15
Mirrors: #4944 (auth-gate deny → resume + continue) — MERGED to main as
c3b4a4d94. The auth disposition plumbing this plan generalizes is already
on main; this is not a stacked branch. (Local checkout 883c576f6 predates
the merge — pull main before implementing.)

## Problem

Approval-gate denial in Reborn cancels the run instead of telling the
model. When a `BlockedApproval` run is denied:

- `DefaultApprovalInteractionService::deny_gate`
  (`crates/ironclaw_product_workflow/src/approval_interaction/service.rs:290`)
  marks the approval record `Denied`, then calls
  `turn_coordinator.cancel_run(...)` and returns
  `ResolveApprovalInteractionResponse::Denied(CancelRunResponse)`.
- The idempotent replay arm `replay_denied_gate` (service.rs:351) also
  `cancel_run`s.
- The WebUI facade maps BOTH resolutions to deny:
  `WebUiGateResolution::Denied | ::Cancelled => ApprovalInteractionDecision::Deny`
  (`crates/ironclaw_product_workflow/src/reborn_services.rs:3105`).

This is the same loop class #4944 just removed for auth gates: the run
terminates `Cancelled`, the model never learns the user declined the
action, and the next trigger re-issues the same approval-gated
capability and re-blocks. The user-visible symptom (extension-install
flow): pressing Deny/Cancel "didn't return anything from the LLM and
just put me in the same loop."

## Goal

Mirror #4944 for approval gates: denial RESUMES the parked run carrying
a denial disposition. The capability stage converts ONLY the
approval-gated call into a model-visible non-retryable failure
(Authorization / `SameCallRetryConstraint::Forbidden`) and the loop
continues. Other parallel calls in the same batch are unaffected.

Non-goal: changing approval *approve* behavior, persistent
always-allow policy, lease issuance, or signing/attested approvals.

## Existing reusable plumbing (from #4944)

- `ResumeTurnRequest` already carries a typed disposition for auth
  (`auth_resume_disposition`); `TurnRunRecord`/`TurnRunState` persist it
  via serde-default (no DB migration — run record is a JSON blob).
- `PendingApprovalResume` already exists as the approval analog of
  `PendingAuthResume` (`crates/ironclaw_agent_loop/src/state.rs:101`),
  and the capability stage already gates approval-resume by
  `capability_id` (the approval match path the #4944 reviewers cited at
  capabilities.rs:212).
- The capability-stage denied-resume short-circuit pattern (partition
  `visible_calls` by `capability_id`, synthesize one Forbidden failure
  via `handle_capability_error` with empty `safe_summary` + planner
  summary, recompute batch policy after partition, single-ownership
  take) is already implemented for auth and is the template to reuse.
- Durable `ApprovalStatus::Denied` already exists in run-state
  (`crates/ironclaw_run_state/src/lib.rs:67`).

## Proposed change

1. **Carrier (UNIFIED — resolves Q1 per maintainability review).** Do
   NOT add a parallel `ApprovalResumeDisposition`. Instead rename
   `AuthResumeDisposition` → `GateResumeDisposition` (one shared enum in
   `ironclaw_turns`, unit variant `Denied`) and collapse
   `ResumeTurnRequest.auth_resume_disposition` → `resume_disposition:
   Option<GateResumeDisposition>`. Both `PendingAuthResume.disposition`
   and the new `PendingApprovalResume.disposition` become
   `Option<GateResumeDisposition>`. The `precondition`
   (`BlockedAuthGate` vs `BlockedApprovalGate`) already carries the
   gate-kind branch, so the disposition needs no per-kind type. Two
   consumers exist today (auth + approval) — this is de-duplication, not
   speculative generality. Persisted on `TurnRunRecord`/`TurnRunState`
   via serde-default; the rename is serde-compatible only if the wire
   tag is held stable (keep `#[serde(rename = "auth_resume_disposition")]`
   on the field, or add a `#[serde(alias)]`, and add a legacy-JSON
   round-trip test — the field is in persisted run records).

2. **Service.** Replace `deny_gate`'s `cancel_run` with a
   `resume_denied_approval` that:
   - marks the approval record `Denied` (unchanged durable write), then
   - `resume_turn(precondition = BlockedApprovalGate,
     approval_resume_disposition = Some(Denied))`, returning
     `ResolveApprovalInteractionResponse::Resumed(ResumeTurnResponse)`.
   - `replay_denied_gate` becomes an idempotent resume with the same
     terminal-run guard #4944 added (`TurnStatus::is_terminal()` →
     Cancelled maps to a cancelled response, other terminal → StaleGate,
     non-terminal + disposition present → Resumed).

3. **Executor (SHARED HELPER — resolves code-judo review).**
   `PlannedDriver::resume` stamps `pending_approval_resume.disposition`
   from `request.resume_disposition`. Do NOT copy the auth denied
   short-circuit a second time. Extract the existing auth block in
   `CapabilityStage::process` into one private helper
   `short_circuit_denied_resume(state, denied_capability_id,
   planner_summary, batch)` that: partitions `visible_calls` by the
   denied `capability_id`, fails only the matching call with
   `CapabilityFailureKind::Authorization` + empty `safe_summary`,
   recomputes batch policy after partition, consumes the disposition
   once. Both the auth path and the approval path call it; each call
   site is one `if disposition == Denied` check plus the shared call.
   Planner summary: `"approval gate denied by user"` (mirrors the auth
   sibling `"auth gate denied by user"`; passes `validate_loop_safe_summary`
   since it contains neither `"authorization:"`).

4. **Response type.** `ResolveApprovalInteractionResponse` gains
   `Resumed(ResumeTurnResponse)`; `Denied(CancelRunResponse)` is removed
   (mirrors #4944 L-b deleting `DenialResumed`). Callers in
   `workflow.rs` / `reborn_services.rs` collapse to the resumed path.

5. **Cancel vs Deny.** Keep both `WebUiGateResolution::Denied` and
   `::Cancelled` mapping to `ApprovalInteractionDecision::Deny` →
   resume + continue, matching #4944's final decision and the user's
   stated intent ("approval cancel also doing the same"). No new
   `Cancel` decision variant.

## Separate but related: extension-install observation gap

Even with deny→continue, the resumed model stays blind if the tool
result carries no model-visible observation. `extension_install` /
`extension_search` return results that hit the
`append_capability_safe_summary_ref` path
(`crates/ironclaw_agent_loop/src/executor/capabilities.rs:870`) with
`model_observation = None` (capabilities.rs:807), so the model sees
"safe summary only" and re-calls. This is an orthogonal
missing-observation bug, not fixed by the deny path. Tracked
separately; the deny-continue plan does not depend on it, but the
extension-install loop is only fully resolved when both land.

## Decisions

- **Q1 — RESOLVED (maintainability review).** Use one unified
  `GateResumeDisposition`, not a parallel approval-specific enum. See
  Step 1.
- **Q2 — RESOLVED.** `ironclaw_agent_loop` must not depend on the
  approval store (product_workflow boundary), so carry the disposition
  via the resume request rather than reading durable `ApprovalStatus`
  in the executor.
- **Q3 — RESOLVED (maintainer). Both continue; deny/cancel unified.**
  Every *gate resolution* the UI sends now resumes the run: the approval
  card sends `denied`; the auth cards send `cancelled` (their only
  negative = "won't provide credential"). Neither gate has a separate
  stop button — **run termination is the separate X → `cancelRun` route**,
  not a gate resolution. Because `Denied` and `Cancelled` are therefore
  treated identically everywhere, the two `WebUiGateResolution` variants
  were unified into a single **`Declined`** variant (serde aliases
  `"denied"`/`"cancelled"` keep the wire stable; no JS change). Facade
  maps `Declined → Deny` for both auth and approval. No `Cancel` decision
  variant — stopping is `cancelRun`, already separate.
- **#6 (WebUI) — RESOLVED.** `useChat.resolveGate` previously kept
  processing only for `approved`/`credential_provided`, dropping
  processing + `activeRun` on `denied`/`cancelled` — but those now
  resume. Fixed: `resolveGate` always keeps processing/`activeRun` (the
  terminal `run_status` SSE event clears it; the X/`cancelRun` is the
  only stop). Also fixes the latent auth-`cancelled` desync from #4944.
- **Q4 — RESOLVED (maintainer). Separate PRs, rollout-gated together.**
  This plan (PR1) ships only approval deny → continue. The
  `extension_install`/`extension_search` model-visible observation fix
  is a separate PR2. Neither enables the user-visible extension-install
  path until BOTH land — gate the rollout so deny-continue cannot
  produce a new blind loop on its own. PR1 must carry a note that the
  extension-install loop is not closed until PR2 ships.

## Test plan

- Service: parked approval gate + Deny resumes (not cancels); idempotent
  replay against terminal vs non-terminal run; get_run_state error path.
- Executor: denied approval-resume fails only the matching capability,
  unrelated parallel calls proceed; batch policy recomputed; disposition
  consumed once; checkpoint round-trip of
  `Some(GateResumeDisposition::Denied)`.
- Serde: `ResumeTurnRequest` / `TurnRunRecord` missing-field defaults to
  None.
