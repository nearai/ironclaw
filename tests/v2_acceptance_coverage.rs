//! Engine v2 acceptance test coverage scaffold (#2800 PR-D).
//!
//! The #2800 tracker lists six coverage gaps that must be closed before
//! engine v2 can become the default:
//!
//! 1. gate pause/resume
//! 2. auth flow round-trip
//! 3. mission lifecycle end-to-end
//! 4. retrieval/learning flows (project-scoped docs in context)
//! 5. orchestrator-driven compaction
//! 6. broader replay parity with recorded traces
//!
//! Each is represented here as `#[ignore]` stubs with detailed TODOs
//! pointing at the harness + assertions required. As each is implemented:
//! - remove `#[ignore]`, delete the TODO
//! - check off the corresponding line in issue #2800
//!
//! These stubs keep the surface tracked — `cargo test -- --ignored` shows
//! exactly what's still pending, instead of a blank directory where the
//! gap used to live.

#![allow(dead_code)]

// ── 1. Gate pause / resume ────────────────────────────────────────────

/// Covers #2800 PR-D item 1: `WriteExternal` / `Financial` effects must
/// pause the thread via `ThreadOutcome::GatePaused` and resume cleanly
/// through the unified gate resolver.
///
/// TODO(#2800 PR-D item 1):
/// 1. Use `TestRig` + `TraceLlm` to drive a Thread through an action
///    with `EffectType::WriteExternal`.
/// 2. Assert `ThreadOutcome::GatePaused` is produced.
/// 3. Resolve via `POST /api/chat/gate/resolve` with approved=true.
/// 4. Assert the thread resumes and completes normally.
/// 5. Negative variant: approved=false → thread fails cleanly with a
///    recognizable reason string.
#[tokio::test]
#[ignore = "acceptance coverage — #2800 PR-D gate pause/resume"]
async fn v2_write_external_gate_pauses_and_resumes() {
    unimplemented!("see TODO in test comment");
}

// ── 2. Auth flow round-trip ───────────────────────────────────────────

/// Covers #2800 PR-D item 2: a tool requiring a missing credential must
/// surface an auth gate, complete through the OAuth/token store flow,
/// and the paused thread must resume with the credential now available.
///
/// TODO(#2800 PR-D item 2):
/// 1. Seed no credential for user A.
/// 2. Drive a Thread through an action requiring `credential_name`.
/// 3. Assert GatePaused with a credential-auth shape.
/// 4. Simulate the auth callback via
///    `handle_engine_auth_callback(state, ...)`.
/// 5. Assert the Thread resumes, makes the same action call again, and
///    this time the credential is present.
#[tokio::test]
#[ignore = "acceptance coverage — #2800 PR-D auth flow round-trip"]
async fn v2_auth_gate_completes_round_trip_to_resume() {
    unimplemented!("see TODO in test comment");
}

// ── 3. Mission lifecycle end-to-end ────────────────────────────────────

/// Covers #2800 PR-D item 3: full Mission lifecycle (create → list →
/// fire → complete) must work end-to-end.
///
/// TODO(#2800 PR-D item 3):
/// 1. Create a Mission via `mission_create` action.
/// 2. List missions, assert the new one is present.
/// 3. Fire it manually via `mission_fire`.
/// 4. Assert a child Thread is spawned and runs to completion.
/// 5. Complete via `mission_complete`.
/// 6. Assert the mission is marked Completed and further fires no-op.
#[tokio::test]
#[ignore = "acceptance coverage — #2800 PR-D mission lifecycle"]
async fn v2_mission_lifecycle_end_to_end() {
    unimplemented!("see TODO in test comment");
}

// ── 4. Retrieval / learning (project docs in context) ──────────────────

/// Covers #2800 PR-D item 4: memory docs in a project must surface in
/// the system prompt during thread execution via `build_step_context`.
///
/// TODO(#2800 PR-D item 4):
/// 1. Pre-seed a `DocType::Lesson` MemoryDoc in a project.
/// 2. Spawn a Thread in that project with a goal keyword-matching the doc.
/// 3. Use a `TraceLlm` that captures the system prompt passed on each call.
/// 4. Assert the "## Prior Knowledge" section is present in the first
///    LLM call's system prompt and contains the seeded doc's title.
#[tokio::test]
#[ignore = "acceptance coverage — #2800 PR-D retrieval injection"]
async fn v2_memory_docs_surface_in_system_prompt() {
    unimplemented!("see TODO in test comment");
}

// ── 5. Orchestrator-driven compaction ──────────────────────────────────

/// Covers #2800 PR-D item 5: the Python orchestrator at
/// `crates/ironclaw_engine/orchestrator/default.py:240-310` must compact
/// working messages when token count crosses 85% of the model limit.
///
/// TODO(#2800 PR-D item 5):
/// 1. Stub the context estimator so 85% fires on a small message count.
/// 2. Configure a Thread to use CodeAct / scripting tier.
/// 3. Pump enough messages to cross the threshold.
/// 4. Assert the orchestrator state's `working_messages` got replaced by
///    `[system, summary_msg, continuation_prompt]`.
/// 5. Assert the snapshot ends up in state history for audit.
/// 6. Assert FINAL() still resolves correctly after compaction.
#[tokio::test]
#[ignore = "acceptance coverage — #2800 PR-D orchestrator compaction"]
async fn v2_orchestrator_compaction_preserves_intermediate_results() {
    unimplemented!("see TODO in test comment");
}

// ── 6. Replay parity ───────────────────────────────────────────────────

/// Covers #2800 PR-D item 6: recorded traces under
/// `tests/fixtures/llm_traces/` must replay identically on v2.
///
/// TODO(#2800 PR-D item 6):
/// 1. For each trace in `tests/fixtures/llm_traces/`, replay through
///    `TraceLlm` + `TestRig.with_engine_v2()`.
/// 2. Assert the same final outcome (text, tool calls, error-free) as v1.
/// 3. When `ENGINE_V2_RELIABILITY_HINTS=true`, `TraceLlm` must tolerate
///    drift in the "## Action reliability notes" system-prompt block —
///    that section is non-deterministic across runs and must not fail
///    replay parity.
#[tokio::test]
#[ignore = "acceptance coverage — #2800 PR-D replay parity"]
async fn v2_replays_recorded_traces_with_identical_outcome() {
    unimplemented!("see TODO in test comment");
}

// ── PR-differential: v2 is strictly better than v1 ─────────────────────
//
// Each test below runs the same input through v1 and v2, asserting v2
// is at least as good on a measurable axis and strictly better on at
// least: success rate, token cost, turn count, one security property.
//
// Differential scope was agreed in #2800. These tests gate the final
// default-flip merge (see the "Rollout gates" section in the tracker).

/// Differential: task success rate on multi-tool-call fixtures.
/// Expect v2 ≥ v1 (v2's CodeAct can compose; v1 does one tool per turn).
#[tokio::test]
#[ignore = "differential — #2800 task success rate"]
async fn differential_v2_success_rate_at_least_v1() {
    unimplemented!("see TODO in test comment");
}

/// Differential: token cost on retrieval-heavy tasks.
/// Expect v2 ≤ v1 (project-scoped memory docs vs full history replay).
#[tokio::test]
#[ignore = "differential — #2800 token cost"]
async fn differential_v2_token_cost_at_most_v1() {
    unimplemented!("see TODO in test comment");
}

/// Differential: turn count on multi-step workflows.
/// Expect v2 < v1 on fixtures needing 5+ tool calls (CodeAct batches).
#[tokio::test]
#[ignore = "differential — #2800 turn count"]
async fn differential_v2_turn_count_lower_on_multi_step() {
    unimplemented!("see TODO in test comment");
}

/// Differential: recursive sub-agent capability (v2-only).
/// A task decomposable into parallel independent sub-queries: v2 solves
/// it in one step via `llm_query_batched`; v1 either sequentializes or
/// fails. This is a "v1 cannot cross the bar" test.
#[tokio::test]
#[ignore = "differential — #2800 recursive sub-agent"]
async fn differential_v2_only_recursive_sub_agent_capability() {
    unimplemented!("see TODO in test comment");
}

/// Differential: learning across threads.
/// Run the same task twice. v2 produces a `DocType::Lesson` between runs
/// and completes the second run in fewer turns. v1 has no learning loop
/// — identical turn count run-to-run. Assert v2 improves.
#[tokio::test]
#[ignore = "differential — #2800 learning across threads"]
async fn differential_v2_improves_run_over_run_via_lessons() {
    unimplemented!("see TODO in test comment");
}

/// Differential: capability lease expiry is a v2-only security property.
/// A time-limited lease (5s) blocks the action after expiry; v1's static
/// `ApprovalRequirement` has no equivalent — asserting it "blocks" is
/// meaningless. This test documents v2's strictly-better safety stance.
#[tokio::test]
#[ignore = "differential — #2800 lease expiry"]
async fn differential_v2_capability_lease_blocks_after_expiry() {
    unimplemented!("see TODO in test comment");
}

/// Differential: provenance taint on `Financial` effects from LLM output.
/// v2 `PolicyEngine::evaluate_with_provenance` forces `RequireApproval`;
/// v1 allows by default. Strictly-better safety.
#[tokio::test]
#[ignore = "differential — #2800 provenance taint"]
async fn differential_v2_taints_llm_generated_financial_effects() {
    unimplemented!("see TODO in test comment");
}

/// Differential: event-sourced determinism.
/// Same trace + same seed produces identical `ThreadEvent` sequences in
/// v2. v1 has no event sourcing → test that v1 can't pass, documenting
/// the v2-only capability.
#[tokio::test]
#[ignore = "differential — #2800 event sourcing determinism"]
async fn differential_v2_replay_is_deterministic_per_event_log() {
    unimplemented!("see TODO in test comment");
}

/// Differential: long-context graceful degradation.
/// Feed > 85% of context window. v2 compacts via orchestrator; v1 either
/// errors or silently truncates. Assert v2 completes the task, v1 fails
/// or loses information.
#[tokio::test]
#[ignore = "differential — #2800 long-context compaction"]
async fn differential_v2_compacts_long_context_v1_fails() {
    unimplemented!("see TODO in test comment");
}

/// Differential: reliability-driven tool avoidance (after PR-B wiring).
/// Pre-warm tracker with 10 failures of tool X. Submit a task where X is
/// one of several valid choices. Assert v2 prefers an alternative; v1
/// has no such mechanism.
#[tokio::test]
#[ignore = "differential — #2800 reliability avoidance"]
async fn differential_v2_avoids_known_unreliable_tools() {
    unimplemented!("see TODO in test comment");
}

/// Differential: parallel thread isolation within a project.
/// Spawn two threads in the same project with overlapping leases. Assert
/// both complete and memory writes are project-scoped and don't
/// interfere. v1's session model doesn't express this shape.
#[tokio::test]
#[ignore = "differential — #2800 parallel thread isolation"]
async fn differential_v2_parallel_threads_isolated_within_project() {
    unimplemented!("see TODO in test comment");
}

/// Differential: CodeAct error self-correction.
/// A script raises `NameError`, then self-corrects on the next iteration.
/// Assert v2 reaches FINAL(). v1 treats tool errors as terminal at the
/// same position. Measures recovery rate on synthetic broken-code fixtures.
#[tokio::test]
#[ignore = "differential — #2800 CodeAct self-correction"]
async fn differential_v2_codeact_recovers_from_python_errors() {
    unimplemented!("see TODO in test comment");
}

/// Differential: cost-budget pre-call enforcement.
/// Thread with `max_budget_usd = 0.05`. v2 terminates cleanly via
/// `cost_guard_gate` before spending more. v1 has no pre-call gate (only
/// post-fact accounting). Strictly-better safety.
#[tokio::test]
#[ignore = "differential — #2800 cost budget enforcement"]
async fn differential_v2_pre_call_cost_budget_enforcement() {
    unimplemented!("see TODO in test comment");
}
