# Slack triggered-run delivery: treat parked-awaiting-user runs as terminal-for-delivery

Date: 2026-06-25
Branch: `fix/reborn-slack-delivery-blocked-terminal`
Owner: Reborn

## Summary

Triggered-run Slack delivery records `Failed` for runs that park in
`BlockedApproval` / `BlockedAuth` (awaiting user approval or re-auth) and never
resolve. Production logs show **23 such `Failed` outcomes in ~1.5h**, each
preceded by a `RunWaitTimedOut` after the **30-minute** delivery backstop, with
**zero** Slack posts in between. The fix: once the delivery loop has delivered a
user-actionable gate/auth prompt, a subsequent wait timeout means the run is
*parked awaiting the user* — that is a **successful (Delivered) terminal outcome
for delivery**, not a failure. The 30-minute backstop stays as the failure mode
only for runs that never reach an actionable state (genuinely still running).

## Verified evidence (not assumed)

Log file: `~/Downloads/logs.1782348290172.log` (ANSI-stripped).

- `23` × `WARN ... slack_delivery: triggered run wait failed ... did not finish before Slack delivery timeout`.
- All `23` delivery outcomes are `outcome=Failed`. Zero `Delivered`/`Skipped`.
- Blocked-state counts in window: `21` × `status=BlockedApproval`, `3` × `status=BlockedAuth` (the latter from `lease_once failed err=secret expired` → `error_kind="AuthRequired"` on `google-calendar.list_events`).
- Exact 30-min poll confirmed on individual runs:
  - `58ddc152`: `status=BlockedApproval` at `23:15:39` → `RunWaitTimedOut` at `23:45:45` (30m06s) → `outcome=Failed`.
  - `9461329b`: `status=BlockedAuth` at `23:30:24` → `RunWaitTimedOut` at `00:00:29` (30m05s) → `outcome=Failed`.
- **Zero** `chat.postMessage` / egress events anywhere in the log.

Mechanism reproduced with an in-tree test (`ScriptedTurnCoordinator` returning a
sticky `BlockedApproval`, `max_wait=200ms`): the approval prompt posts **once**,
then the loop re-waits, times out, and records **`Failed`**, overwriting the
`Delivered` it had already earned. `record_triggered_run_delivery` uses
`CasExpectation::Any` (blind overwrite), so the later `Failed` clobbers the
earlier `Delivered`.

### Code path (verified)

`crates/ironclaw_reborn_composition/src/slack_delivery.rs`:

- `deliver_triggered_run` (l.2012) loops: `wait_for_actionable_triggered` → build
  notification → deliver.
- After a successful **blocked** delivery (`ApprovalNeeded` / `AuthRequired`,
  l.2180-2192) it sets `delivered_blocked_marker = Some(marker)` and `continue`s.
  This re-wait is **intentional**: it lets the loop deliver the eventual
  `Completed` final reply and delete the now-stale OAuth prompt
  (`messages_to_delete_after_final`) once the user acts.
- `wait_for_actionable_triggered` (l.2309) returns early on `is_terminal()` or a
  **new** `blocked_actionable_marker`. When the run stays in the **same** blocked
  state (same gate_ref), the marker equals `delivered_blocked_marker`, so it does
  NOT return — it polls to `settings.max_wait` and returns
  `Err(RunWaitTimedOut)`.
- The outer loop's wait-error arm (l.2079-2089) unconditionally records
  `Failed`. This is the defect: a parked-awaiting-user run is recorded as a
  delivery failure even though the actionable prompt was already delivered.
- Triggered path `max_wait = DEFAULT_TRIGGERED_RUN_DELIVERY_MAX_WAIT = 30 min`
  (l.60, set in `TriggeredRunDeliveryDriver::new`, l.1853-1869).

`get_run_state` and the store are correct: executor and delivery share the same
`Arc<FilesystemTurnStateStore>` (verified in `slack_host_beta.rs:507`,
`Arc::clone(&parts.turn_coordinator)`, and the runtime wiring), the blocked
status is persisted (`block_claimed_record` sets status + gate_ref,
`prune_terminal: false`), and the read cache TTL is 500ms. The bug is **not** a
stale read or a store split.

## Regression determination

**NOT a genuine regression.** The entire blocked-delivery machinery
(`wait_for_actionable_triggered`, `blocked_actionable_marker`, the 30-min
`DEFAULT_TRIGGERED_RUN_DELIVERY_MAX_WAIT`) was introduced together in commit
`1e50ddfee` (#4948). No prior version delivered parked runs correctly and then
broke — this is a **latent design defect present since the feature shipped**: the
loop was built to wait for a blocked run to *change* state, but the common case
(user never approves/re-auths within 30 min) was never handled, so it falls
through to the backstop and records `Failed`.

Per the coordinator's instruction, because this is not a regression we skip the
"regression-narrative" mandate (no new e2e/CLAUDE.md regression rule is
*required*). A focused caller-level test for the new behavior is added (it fails
on the old behavior, passes on the new). A short guardrail note is added anyway —
it is cheap and captures a real invariant.

## Fix (minimal; one owner)

Single, contained change in `deliver_triggered_run` / `wait_for_actionable_triggered`.

**Decision: reuse `Delivered`; do NOT add a new outcome variant.**
`TriggeredRunDeliveryOutcomeKind::Delivered` is already documented as "the final
reply **(or gate prompt)** was delivered successfully"
(`crates/ironclaw_outbound/src/triggered_run_delivery.rs:30`). Posting the
actionable gate/auth prompt *is* a successful delivery; the run then parks
awaiting the user, whose resolution arrives via a separate inbound event. Adding
a new "AwaitingUser"/"Parked" variant would ripple through the store schema,
WebUI automation filters, and serialized records for no behavioral gain.

**Change:** Make the wait-timeout outcome depend on whether an actionable prompt
was already delivered.

- `wait_for_actionable_triggered` returns a typed result that distinguishes
  "timed out" from other errors (it already returns
  `Err(RunWaitTimedOut { run_id })`, so the call site can match on it).
- In `deliver_triggered_run`, on `Err(RunWaitTimedOut)`:
  - If `delivered_blocked_marker.is_some()` → the actionable prompt is already
    out and the run is parked awaiting the user. Record **`Delivered`** and
    return. Do NOT clobber with `Failed`. (Do not delete
    `messages_to_delete_after_final` here — the OAuth prompt must remain
    actionable until the user completes or it expires.)
  - If `delivered_blocked_marker.is_none()` → the run never reached an actionable
    state within the backstop (genuinely still running / stuck). Keep current
    behavior: record `Failed` and `warn!`.
- Any other wait error keeps current behavior (record `Failed`).

This is the smallest change that fixes the clobber, preserves the OAuth/approval
multi-stage delete-on-completion path (when the user *does* act before the
backstop), and keeps the 30-min backstop as the failure signal for never-actionable
runs.

### Why not "treat Blocked* as immediately terminal and stop the re-wait entirely"

The re-wait is load-bearing for the OAuth flow: when the user completes auth, the
loop must deliver the final reply and *delete* the stale OAuth prompt. Removing
the re-wait would regress that. We only change the *timeout* disposition of the
re-wait, not the re-wait itself.

### Logging

Use `debug!` (not `info!`/`warn!`) for the new "parked awaiting user; recording
Delivered" path — it is an expected, common, non-error outcome and must not
corrupt the REPL/TUI. Keep the existing `warn!` only for the genuine
never-actionable backstop failure.

## Tests (drive the caller — `deliver_triggered_run` via the driver)

In `slack_delivery.rs` tests (gated `--features slack-v2-host-beta`):

1. `triggered_persistent_blocked_approval_records_delivered_not_failed`
   (red→green): `ScriptedTurnCoordinator` returns sticky `BlockedApproval`
   (never resolves). Seed personal DM preference + egress OKs.
   `max_wait` small (e.g. 200ms), `poll_interval` 1ms. Assert:
   - the approval prompt was posted **≥1** time, AND
   - the final recorded outcome is **`Delivered`** (NOT `Failed`).
   This test **fails on `main`** (records `Failed`) and **passes** after the fix.

2. `triggered_blocked_then_completed_still_delivers_final_reply` (guard the
   preserved path): script `[BlockedApproval{gate}, Completed]` with a finalized
   assistant message seeded. Assert the final reply is delivered and outcome is
   `Delivered` (ensures the re-wait → complete path is intact). (The existing
   `triggered_oauth_auth_to_personal_dm_posts_authorization_url` already covers
   the OAuth two-stage path; keep it green.)

3. `triggered_never_actionable_run_times_out_failed` (backstop preserved):
   `ScriptedTurnCoordinator` sticky `Running` (never blocks, never completes),
   small `max_wait`. Assert outcome `Failed` and **zero** posts — proving the
   genuine-backstop path is unchanged and only the *post-actionable* timeout
   flips to `Delivered`.

## Guardrail note

The invariant lives as a doc comment directly on `deliver_triggered_run`
(in-lane: `slack_delivery.rs` only — no shared/doc files touched):

> Triggered-run delivery treats a run parked in `BlockedApproval`/`BlockedAuth`
> after its actionable gate/auth prompt has already been delivered as
> **terminal-for-delivery = Delivered**. The 30-minute wait backstop is the
> failure signal ONLY for runs that never reached an actionable state; never
> record `Failed` for a parked-awaiting-user run whose prompt was already posted.

## File lane

This change touches ONLY `crates/ironclaw_reborn_composition/src/slack_delivery.rs`
(+ its tests) and this plan doc. No edits to `ironclaw_turns/src/status.rs`,
`ironclaw_reborn/src/runtime.rs`, `turn_scheduler.rs`, or any config — those are
owned by sibling tracks.

## Out of scope / follow-ups

- **`TurnStatus::wait_class()` convergence (separate track):** a sibling track is
  introducing a canonical `TurnStatus` classifier (`wait_class()`) for
  terminal/blocked/active. This fix keeps its parked-state check local to
  `slack_delivery.rs` (`delivered_blocked_marker.is_some()` + matching on the
  existing `RunWaitTimedOut` variant) to stay in-lane and avoid duplicating the
  classifier in a shared file. Converging the local check onto `wait_class()`
  once it lands is a documented follow-up, not part of this PR.

- The `CasExpectation::Any` blind overwrite in `record_triggered_run_delivery`
  is the reason the late `Failed` clobbers `Delivered`. We fix the *caller* (stop
  recording the spurious `Failed`); tightening the store to reject
  terminal→worse transitions is a separate hardening follow-up, not required here.
- Production "zero posts" vs repro "one post": diagnostic only. The fix is
  identical in both readings — a parked run must not be recorded `Failed`. (A
  follow-up may confirm whether prod runs also lack a configured DM target, which
  would be a distinct `NoDefaultConfigured` concern, not this clobber.)
