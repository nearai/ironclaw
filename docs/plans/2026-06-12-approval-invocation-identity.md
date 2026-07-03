# Approval Survival Across Capability Re-Dispatch (Slack re-approval loop fix)

**Date:** 2026-06-12
**Status:** draft for review (revised after plan-mode review: approach/local-patterns/maintainability)
**Related:** #4747 (pending-resume unification), plan #4539 (idempotency enforcement), PR #4799/#4811 (Slack gate routing/feedback), docs/plans/2026-06-10-auth-gate-resume-redispatch.md (introduced `PendingAuthResume`)

## Problem

When a Reborn capability invocation requires both a one-shot approval and a credential
(e.g. `gmail.get_message` needing Google OAuth), every resume cycle demands a brand-new
human approval. Observed on Slack QA: four consecutive approval gates
(e7ffe6b4 → 8f53616a → b3b5f9bb → 7b3e5dab) for one logical capability call.

Root cause chain (all verified against main):

1. Approval resume itself is correct: `resume_json` validates the run/approval/fingerprint,
   finds the one-shot lease, and injects `lease.grant` into the authorized context
   (`crates/ironclaw_capabilities/src/host.rs:669-689`).
2. The dispatched capability then discovers the missing credential →
   `RuntimeCapabilityOutcome::AuthRequired` → `GateStage(Auth)` blocks the run and saves
   `pending_auth_resume` (`crates/ironclaw_agent_loop/src/executor/gates.rs:79-88`).
3. When the run later resumes, the loop re-dispatches the capability from
   `pending_auth_resume` as a **fresh invocation**: `invocation_context_from_visible`
   mints `InvocationId::new()` (`crates/ironclaw_loop_support/src/capability_port.rs:1629`).
4. The fresh invocation cannot satisfy `has_matching_approval_grant`
   (`crates/ironclaw_reborn_composition/src/profile_approval_authorization.rs:159-190`):
   the visible-request grants are empty (`CapabilitySet::default()`,
   `crates/ironclaw_reborn_composition/src/product_live_adapters.rs:446-456`), and the
   prior lease is unusable — `max_invocations = 1` (consumed) and matched by
   `lease.scope == context.resource_scope` where the scope embeds the old `invocation_id`
   (`crates/ironclaw_capabilities/src/helpers.rs:95-99`).
5. → `Decision::RequireApproval` with a fresh `ApprovalRequestId::new()` → new gate.
   Model-visible authorization failures additionally cause the model to re-emit the tool
   call (fresh invocation by definition), same effect.

The same hole exists for the *legitimate* flow: user completes OAuth →
`dispatch_turn_gate_resume` resumes the run (`BlockedAuthGate` precondition,
`crates/ironclaw_product_workflow/src/auth_continuation.rs:99-110`) → re-dispatch mints a
new invocation → a second approval is demanded for an action the user already approved.

Persistent "Always allow" policies (`persistent_approval_grant` branch,
`profile_approval_authorization.rs:176-178`) are exempt — only one-shot approvals break.

## Constraints

- WebUI approval/auth flows currently work and must not regress. WebUI resolves gates
  through the same `ApprovalInteractionService` / `AuthInteractionService` workflow
  services (`crates/ironclaw_product_workflow/src/approval_interaction/service.rs`,
  `auth_interaction/service.rs`); the fix must live below that boundary (loop executor /
  capability host), not in any channel adapter.
- `ironclaw_capabilities` guardrail: "Approval resume must validate and claim the matching
  fingerprinted lease before dispatch" (crates/ironclaw_capabilities/CLAUDE.md). The fix
  must not weaken fingerprint validation or widen what an approval authorizes.
- No parallel authorization path (`CapabilityHost` is the single caller-facing authority
  path, same CLAUDE.md).
- Architecture rules: no duplicate dispatch pipelines (.claude/rules/architecture.md §4).

## Fix A (primary): auth re-dispatch preserves invocation identity

When the loop re-dispatches a capability after an auth gate (`pending_auth_resume`), it
must resume the **same logical invocation** instead of minting a new one:

- Extend the existing `PendingAuthResume` slot
  (`crates/ironclaw_agent_loop/src/state.rs:116`, introduced by
  docs/plans/2026-06-10-auth-gate-resume-redispatch.md) to carry the original
  `invocation_id` (as resume token) and, when the invocation had previously passed an
  approval, the original `approval_request_id`. New fields use the established
  `#[serde(default, skip_serializing_if = "Option::is_none")]` pattern so existing
  checkpoints decode (see `checkpoint_payload_without_auth_resume_slot_decodes_to_none`).
- Follow-on (under #4747): collapse `PendingApprovalResume` + `PendingAuthResume` into one
  slot. Note this is a NEW type this plan family introduces — the name
  `PendingCapabilityResume` does not yet exist in the codebase; final name decided in
  #4747. The unification deletes the parallel `pending_*_resume_candidate` /
  `clear_matching_*` helper pairs in the executor, so it must be a net concept deletion
  (two slots, two candidate fns, two clear fns → one each), not a third shape.
- On re-dispatch, route through the existing resume path (`resume_json`-style) with the
  original invocation context, rather than `invoke_json` with a fresh
  `InvocationId::new()`.
- The capability host already validates approval/lease/fingerprint for resumes; an
  auth-resume for an invocation that never needed approval skips the lease step
  (approval_resume = None), preserving current behavior for approval-less capabilities.
- Run-state model: the run record for the invocation transitions BlockedAuth →
  dispatched, mirroring the existing BlockedApproval → dispatched transition
  (`CapabilityRunStateTransition::BlockAuth`, `crates/ironclaw_capabilities/src/helpers.rs:120-131`).

Effect: one logical call = one invocation = one approval, regardless of how many gates
(approval, auth, resource) it crosses.

## Fix B (companion, independent PR): surface credential requirements before approval

Today's per-invocation gate order is: profile approval gate (authorization layer) →
dispatch → credential discovery (AuthRequired). A human approves an action that cannot
execute yet; the approval is then burned by the auth bounce (until Fix A) and the UX is
approve → auth → (re)approve.

Change: surface missing credentials **before** the approval gate in the invocation
pipeline. If credentials are missing, emit `AuthRequired` first; the approval gate fires
only when the call is otherwise executable. Approval becomes the last gate, so approving
means the action runs immediately.

Single source of truth (review finding, duplicate-truth): do NOT add a second
credential-requirements computation. Extract the requirements derivation currently inside
the dispatch path (`auth_required_outcome` inputs,
`crates/ironclaw_host_runtime/src/production.rs:1483-1495`) into one canonical query
function on the capability manifest; both the new pre-flight check and the existing
dispatch-time check call that same function. The dispatch-time check stays as the
enforcement backstop (pre-flight is advisory ordering, not authority).

WebUI note: WebUI renders auth gates and approval gates as distinct cards; reordering
changes only which card appears first, not the wire contract of either gate
(`RunNotificationEventKind::AuthRequired` / `ApprovalNeeded` unchanged).

## Fix C (defense in depth, separate plan #4539): enforce the advisory idempotency key

`invocation_idempotency_key` (`crates/ironclaw_loop_support/src/capability_port.rs:1772-1799`)
is stable across identical retries (`loop-capability:sha256:…`, confirmed in QA logs) but
only logged today. Goal: a model-retry of the identical call (genuinely new tool call,
identical input) within the same run reuses the human's earlier approval. Covers the
variant Fix A does not.

Mechanism (revised per review finding, code-judo): do NOT add a new approved-request
lookup index. Express the reuse as a **run-scoped policy entry through the existing grant
path**: when an approval is granted, additionally store a constrained policy —
`max_invocations: Some(1)` per reuse, `expires_at` bounded to the run, constraint keyed by
the invocation-independent input fingerprint — such that the existing
`has_matching_approval_grant` matching
(`crates/ironclaw_reborn_composition/src/profile_approval_authorization.rs:159-190`) and
the `PersistentApprovalPolicyInput` shape
(`crates/ironclaw_product_workflow/src/approval_interaction/service.rs:249-265`, today
hardcoding `max_invocations: None`) absorb it. One approval-reuse mechanism, not three.

Scope guard: run-scoped and input-fingerprint-exact; never crosses runs, users, or
inputs; never applies to Deny/Expired requests.

## Sequencing

1. Fix A under #4747 (M) — removes the loop for gate-crossing invocations.
2. Fix B as its own PR (S–M) — UX ordering; independent of A, lands either order.
3. Fix C under plan #4539 (M) — later; requires durable approved-request lookup index.

## Observability

The re-dispatch junction (auth-resume reusing the original invocation) emits a structured
`debug!` event — at minimum `%invocation_id, auth_resume = true, approval_request_id` —
mirroring the existing decision-point tracing in
`crates/ironclaw_capabilities/src/host.rs` ("capability invocation started" /
"capability run state started"). The QA loop was diagnosed from logs; the fix must be
confirmable the same way.

## Testing

- Loop-level: integration test driving approve → AuthRequired → auth-resume → dispatch
  completes WITHOUT a second approval request (extends
  `crates/ironclaw_reborn/tests/loop_driver_host.rs` approval/resume coverage, e.g.
  `turn_runner_blocks_on_approval_then_coordinator_resume_completes_same_run`).
- Capability-host contract: auth-resume with original invocation id reuses run record;
  fingerprint mismatch still rejected.
- Product-workflow contract: WebUI-path approve/auth resolution unchanged
  (`crates/ironclaw_product_workflow/tests/approval_interaction_contract.rs`,
  `auth_interaction_contract.rs`).
- E2E (tests/e2e): Slack DM scenario — approve once, complete OAuth, run completes; no
  duplicate approval gates.

## Open questions

1. Exact trigger that resumed the QA run out of `BlockedAuth` without OAuth completion —
   needs a runtime trace; possibly a premature auth-continuation dispatch. Tracked as a
   separate small bug; Fix A makes the consequence benign either way.
2. Should a one-shot approval survive a *failed* dispatch retry (transient network error →
   `RetrySameCall`)? Proposal: yes when the retry reuses the invocation id (Fix A path),
   no for model-initiated new calls until Fix C.
