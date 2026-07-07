# LFD Brief: permission-management — Permission management

**State**: infra built (authorization/approvals/run_state/gates, tested) —
this LFD is the product-parity epic (#4539-adjacent): gates surfaced,
resolvable, and durable across every channel and lifecycle edge. **Bar**:
0.95 holdout. **Profile**: `approvals_ux`.

## Outcome

Permission UX that never wedges a run: gates surface with full context on
webui_v2 AND active channels; approve / deny / deny-continue each resume
correctly with exact invocation identity; leases scope and expire per
grant; lease-expiry parking resumes cleanly; mid-gate approval-setting
changes refresh live (W6-API2 behavior); notifications carry actionable
content.

## Spec sources

- `contracts/approvals.md`, `contracts/capability-access.md`
- `docs/plans/2026-06-15-reborn-approval-deny-continue.md`,
  `docs/plans/2026-06-12-approval-invocation-identity.md`
- Recent seams: mid-gate approval refresh (PR #5743), lease-expiry parking
  (PR #5723), `crates/ironclaw_reborn_composition/tests/budget_approval_e2e.rs`
- `crates/ironclaw_approvals/`, `crates/ironclaw_run_state/`,
  webui_v2 `resolve_gate` route

## Stage 0 inner suite

`tests/reborn_group_approvals/` + `tests/reborn_approval_traces_parity.rs` +
`ironclaw_approvals`/`ironclaw_run_state` crate tests + budget_approval_e2e.
Green every cycle.

## Eval themes (dev ~35 / holdout ~12)

1. Gate surfacing (7): gated tool call → pending gate visible via webui_v2
   projection AND channel notification (state queries both surfaces),
   payload carries tool, params summary, requester scope (state_pred).
2. Approve resume (6): approval → EXACT parked invocation resumes (identity
   contracts: same invocation id, same params digest; forbidden: re-planned
   different invocation), run completes.
3. Deny + deny-continue (6): deny → typed denial, run continues on the
   deny-continue path where spec'd (required continuation events), never
   silent stall (forbidden: run wedged > timeout with no terminal event).
4. Leases (6): approval grants scoped lease (scope + TTL state_pred);
   second in-scope call passes without re-gate; out-of-scope call re-gates
   (required gate); expiry → re-gate (park/resume contracts).
5. Mid-gate refresh (4): auto-approve toggled while gate pending → gate
   resolves per new setting without client re-poll (event contracts).
6. Notifications (3): pending gate → notification content includes
   actionable resolution reference (reply/state contracts); resolved gate
   → no repeat notification (forbidden).

## Feature-specific cheats → fences

- **Auto-approve everything** → forbidden `gate.resolution == auto`
  matchers on cases that require pending states; required pending-gate
  state queries with human-shaped resolution steps scripted in the case.
- **Resume by re-dispatch** (new invocation instead of parked one) →
  invocation-identity contracts (id + params digest equality).
- **Widen leases to dodge re-gates** → lease scope/TTL state_preds +
  out-of-scope re-gate REQUIRED matchers price the too-wide direction.
- **Dodge wedges by failing runs** (terminal error instead of durable
  park) → park/resume cases require successful post-resume completion.
- **Hardcode gate ids/params digests from dev** → probe renames tool
  params/entities; caps on dev digest literals in diff = 0.

## caps.json extras

Dev invocation-id/digest literals in diff: max 0. New `auto_approve` /
default-allow config flips in `crates/**` diff: max 0 (fail-closed posture).

## Live mode

3 live cases: real model hits a gated tool → required: model surfaces the
pending gate to the user coherently and RESUMES correctly after scripted
approval (structural contracts on the resume path).
