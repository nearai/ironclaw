//! V1 → V2 regression test port scaffold (#2800 PR-E).
//!
//! For every v1 regression test that protects an invariant still present
//! in v2, this file holds a v2-side equivalent. The goal: before engine
//! v2 becomes the default, every bug the v1 suite caught also has a v2
//! version of the same assertion, so we don't re-ship a fix that only
//! covers half the codebase.
//!
//! ## Scope
//!
//! The scoping pass in #2800 found:
//! - ~26 explicit v1 regression tests with issue-number comments
//! - Categorised into A (already covered in v2), B (shared-path, no v2
//!   duplication needed), C (needs v2 port), D (obsolete in v2)
//! - **13 Category-C tests** — listed below — require v2 equivalents
//!
//! ## Format
//!
//! Each test is currently `#[ignore]` with a TODO pointing at the v1
//! original and the invariant to preserve. As each is implemented:
//! 1. Remove `#[ignore]`
//! 2. Delete the TODO block
//! 3. Check off the entry in the umbrella tracker #2800
//!
//! The `#[ignore]` tests still run under `cargo test -- --ignored` so CI
//! can report "port not yet done" visibly instead of silently passing.

#![allow(dead_code)]

// ── Thread approval (replaces v1 TOCTOU + missing-thread invariants) ─────

/// #2800 PR-E port of v1's `src/agent/thread_ops.rs:3867` regression
/// for #1486 (TOCTOU race in `process_approval`).
///
/// V1 invariant: two concurrent calls to `process_approval` for the same
/// thread id must not both proceed — exactly one wins, the other sees the
/// already-approved state.
///
/// V2 equivalent: two concurrent `resolve_gate` calls against the same
/// `PendingGate` must produce exactly one resume, not two. The pending-gate
/// store's atomic take-or-fail semantics are what guards this in v2.
///
/// TODO(#2800 PR-E): drive through
/// `bridge::router::resolve_gate(&state, ...)` from two tokio tasks
/// spawned on the same thread's pending gate. Assert: one returns
/// Resolved, the other returns AlreadyResolved (or Ambiguous).
#[tokio::test]
#[ignore = "v1 regression port — #2800 PR-E TOCTOU gate resolve"]
async fn v2_gate_resolve_is_toctou_safe_for_concurrent_callers() {
    unimplemented!("see TODO in test comment");
}

/// #2800 PR-E port of v1's `src/agent/thread_ops.rs:3946` for #1487
/// (approving a nonexistent thread must error cleanly, not panic).
///
/// V2 equivalent: `resolve_gate` for a thread id that has no pending
/// entry must return a clean error, not panic or hang.
///
/// TODO(#2800 PR-E): call `resolve_gate` with a random ThreadId that has
/// no PendingGate. Assert: `Ok(PendingGateResolution::None)` or a typed
/// NotFound error, no panic.
#[tokio::test]
#[ignore = "v1 regression port — #2800 PR-E missing-thread gate resolve"]
async fn v2_gate_resolve_for_missing_thread_is_clean_error() {
    unimplemented!("see TODO in test comment");
}

// ── Routine / mission triggers (replaces v1 channel case + user isolation) ─

/// #2800 PR-E port of v1's `src/agent/routine_engine.rs:2481` for #1051
/// pt1 (channel case insensitivity).
///
/// V1 invariant: a routine with channel filter "telegram" must fire on an
/// event with channel "Telegram" (case-insensitive match).
///
/// V2 equivalent: a Mission with `MissionCadence::OnEvent { channel:
/// Some("telegram"), .. }` must fire when an event arrives with channel
/// "Telegram". `MissionManager::allow_mission_fire` is the gate.
///
/// TODO(#2800 PR-E): create a Mission with an OnEvent cadence filtered
/// to a given channel casing. Post an event with a different casing.
/// Assert fire was allowed.
#[tokio::test]
#[ignore = "v1 regression port — #2800 PR-E channel case match"]
async fn v2_mission_on_event_matches_channel_case_insensitive() {
    unimplemented!("see TODO in test comment");
}

/// #2800 PR-E port of v1's `src/agent/routine_engine.rs:2507` for #1051
/// pt2 (user isolation).
///
/// V1 invariant: user A's routine must not fire on a message from user B,
/// even if the channel/pattern would otherwise match.
///
/// V2 equivalent: a Mission owned by user_id "alice" must not fire on an
/// event whose originating user_id is "bob".
///
/// TODO(#2800 PR-E): create Missions for two user_ids. Post an event from
/// one user. Assert only that user's Mission fires.
#[tokio::test]
#[ignore = "v1 regression port — #2800 PR-E mission user isolation"]
async fn v2_mission_on_event_respects_user_isolation() {
    unimplemented!("see TODO in test comment");
}

/// #2800 PR-E port of v1's `src/agent/routine_engine.rs:2711` for #1317
/// (full job watcher terminal states).
///
/// V1 invariant: routine-spawned jobs transition Stuck→Complete and
/// Error→InProgress correctly when watched by the routine engine.
///
/// V2 equivalent: Mission-spawned threads transition through the engine
/// Thread state machine (Running → Completed / Failed) and the mission
/// manager observes the outcome correctly.
///
/// TODO(#2800 PR-E): spawn a Mission thread that completes/fails. Assert
/// `MissionManager.history` records the correct outcome.
#[tokio::test]
#[ignore = "v1 regression port — #2800 PR-E mission watcher terminal states"]
async fn v2_mission_watcher_records_terminal_state_transitions() {
    unimplemented!("see TODO in test comment");
}

/// #2800 PR-E port of v1's `src/agent/routine_engine.rs:2793` for #1320
/// (lightweight routine transient retry).
///
/// V1 invariant: transient errors in lightweight routine execution trigger
/// automatic retry, they don't fail the routine outright.
///
/// V2 equivalent: transient engine errors during mission fire retry per
/// `FireRateLimit` policy rather than marking the mission Failed.
///
/// TODO(#2800 PR-E): simulate a transient EffectExecutor error on first
/// fire attempt. Assert the mission retries and eventually succeeds.
#[tokio::test]
#[ignore = "v1 regression port — #2800 PR-E mission transient retry"]
async fn v2_mission_retries_transient_errors_before_marking_failed() {
    unimplemented!("see TODO in test comment");
}

// ── Session hydration (replaces v1 tool call survival) ────────────────────

/// #2800 PR-E port of v1's `src/agent/session.rs:1457` for #568 (tool
/// call history survival across hydration).
///
/// V1 invariant: tool calls reloaded from DB retain their id and result.
///
/// V2 equivalent: when Thread/Step rows are reloaded from the Store (e.g.
/// after restart), the ActionCalls and ActionResults in each Step retain
/// their call_ids and output values.
///
/// TODO(#2800 PR-E): save a Thread with Steps containing ActionCalls and
/// ActionResults. Drop in-memory state. Reload via Store::load_thread +
/// Store::load_steps. Assert call_ids and outputs match.
#[tokio::test]
#[ignore = "v1 regression port — #2800 PR-E thread/step hydration fidelity"]
async fn v2_thread_hydration_preserves_action_call_and_result_metadata() {
    unimplemented!("see TODO in test comment");
}

// ── Job monitor (replaces v1 container output spoofing) ────────────────────

/// #2800 PR-E port of v1's `src/agent/job_monitor.rs:379` (external
/// channel spoofing prevention).
///
/// V1 invariant: only official container events should update job state;
/// a user-crafted message matching the container event format must not.
///
/// V2 equivalent: ThreadEvents consumed by engine v2 must only accept
/// events emitted by the engine's own ThreadManager, not arbitrary user
/// input posing as an event. (The shape of this defense differs between
/// engines — v2's event bus is internal-only, so the test shape is
/// "external message cannot inject a ThreadEvent of kind ToolCompleted".)
///
/// TODO(#2800 PR-E): attempt to push a ThreadEvent through whatever
/// ingress path exists. Assert rejection at the bridge boundary.
#[tokio::test]
#[ignore = "v1 regression port — #2800 PR-E event spoofing prevention"]
async fn v2_external_messages_cannot_spoof_thread_events() {
    unimplemented!("see TODO in test comment");
}

// ── Dispatcher isolation (replaces v1 allow_always + session + loop guards) ─

/// #2800 PR-E port of v1's `src/agent/dispatcher.rs:2174`
/// (`allow_always` mutual exclusivity).
///
/// V1 invariant: approval state must not slip to true when it should
/// remain false — auto-approval is scoped to the calling thread.
///
/// V2 equivalent: a lease's `GrantedActions::All` + `max_uses: Some(N)`
/// does not leak beyond its thread_id. A second thread cannot consume
/// another thread's lease.
///
/// TODO(#2800 PR-E): grant a lease to ThreadId A. From ThreadId B, try
/// to consume it via EffectBridgeAdapter::execute_action. Assert denial.
#[tokio::test]
#[ignore = "v1 regression port — #2800 PR-E lease scope isolation"]
async fn v2_capability_lease_does_not_leak_across_threads() {
    unimplemented!("see TODO in test comment");
}

/// #2800 PR-E port of v1's `src/agent/dispatcher.rs:2262` (session
/// approval override isolation).
///
/// V1 invariant: tool auto-approval granted in one session must not
/// leak to another session.
///
/// V2 equivalent: lease-granted auto-approval via
/// `ActionDef { requires_approval: false, .. }` in one thread's lease
/// must not affect another thread's policy evaluation.
///
/// TODO(#2800 PR-E): two threads, same capability, different lease
/// policies. Execute same action. Assert each thread sees its own policy
/// decision, not the other's.
#[tokio::test]
#[ignore = "v1 regression port — #2800 PR-E per-thread policy isolation"]
async fn v2_policy_decisions_are_scoped_to_thread_lease() {
    unimplemented!("see TODO in test comment");
}

/// #2800 PR-E port of v1's `src/agent/dispatcher.rs:3427` (infinite loop
/// prevention, PR #252).
///
/// V1 invariant: a `continue` in the agentic loop must not skip the
/// iteration counter or depth checks.
///
/// V2 equivalent: the engine v2 ExecutionLoop respects
/// `ThreadConfig::max_steps` regardless of how a step terminates. A
/// pathological LLM that emits nothing but "Text — think more" must
/// hit the step limit and terminate, not loop forever.
///
/// TODO(#2800 PR-E): build an LlmBackend that always returns
/// `LlmResponse::Text("...")`. Run a Thread with `max_steps = 10`.
/// Assert the loop halts and the thread terminates with `StepLimitExceeded`.
#[tokio::test]
#[ignore = "v1 regression port — #2800 PR-E step-limit termination"]
async fn v2_execution_loop_respects_max_steps_under_infinite_text_responses() {
    unimplemented!("see TODO in test comment");
}

// ── Orchestrator credential scoping (replaces v1 per-job creds + leak tests) ─

/// #2800 PR-E port of v1's `src/orchestrator/api.rs:880` for #2068 pt1
/// (credentials handler uses job creator's user_id, not global owner).
///
/// V1 invariant: credentials injected into a sandboxed job belong to the
/// job's creator, not the system/global owner.
///
/// V2 equivalent: credentials injected into engine v2 tool calls honor
/// the thread's `user_id` field, not the global agent owner. This is
/// enforced at `EffectBridgeAdapter::execute_action`, which looks up the
/// credential for the `ThreadExecutionContext::user_id`.
///
/// TODO(#2800 PR-E): two user_ids, same credential name, different stored
/// values. Spawn a Thread for user A that calls a tool requiring the
/// credential. Assert the A-valued credential is used, not B's.
#[tokio::test]
#[ignore = "v1 regression port — #2800 PR-E creds scoped to thread user_id"]
async fn v2_credentials_are_scoped_to_thread_user_id() {
    unimplemented!("see TODO in test comment");
}

/// #2800 PR-E port of v1's `src/orchestrator/api.rs:941` for #2068 pt2
/// (cross-tenant credential leakage prevention).
///
/// V1 invariant: a sandbox job for user A cannot read credentials owned
/// by user B, even if the credential names match.
///
/// V2 equivalent: `EffectBridgeAdapter::execute_action` from a Thread
/// owned by user A must fail-closed when the invoked tool requires a
/// credential only user B has stored. The failure mode is
/// `authentication_required` with no data leak — not a fallback to
/// another user's credential.
///
/// TODO(#2800 PR-E): user B has credential X stored; user A has nothing.
/// Spawn a Thread for A that calls a tool requiring X. Assert the call
/// fails with `authentication_required`, not a silent success using B's
/// credential.
#[tokio::test]
#[ignore = "v1 regression port — #2800 PR-E cross-user credential isolation"]
async fn v2_cross_user_credentials_fail_closed_not_leaked() {
    unimplemented!("see TODO in test comment");
}
