# Auth-Gate Resume Re-Dispatch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When an auth-gated capability call resumes after OAuth completes, the loop re-dispatches the original capability invocation automatically (mirroring approval-gate resume), so the model receives the real payload instead of waking with no signal.

**Architecture:** Add a `pending_auth_resume: Option<PendingAuthResume>` slot to `LoopExecutionState`, populated in `GateStage`'s Block arm for `GateKind::Auth` from the already-available `CapabilityCallCandidate`. On resume, the prompt stage detects the slot (exactly like `pending_approval_resume` at `prompt.rs:239`) and returns a resume step that skips the model call and re-invokes the capability. Unlike approval resume, no resume token is attached — the host re-evaluates the auth requirement fresh; credentials now present → executes and returns the full payload; still missing → blocks again (idempotent).

**Tech Stack:** Rust, `crates/ironclaw_agent_loop` (canonical executor), `crates/ironclaw_turns` (neutral contracts — likely untouched).

**Bug context (diagnosed 2026-06-10):** Auth gates block with `approval_resume: None` (`executor/capabilities.rs:464`), store no resume record, append no tool result (`executor/gates.rs:61-101`). `LoopInput::GateResolved` is never consumed or rendered (`executor/input.rs:186`). After OAuth → `dispatch_turn_gate_resume` (`ironclaw_product_workflow/src/auth_continuation.rs:64`) → `PlannedDriver::resume` (`ironclaw_reborn/src/planned_driver.rs:123`) reloads the checkpoint and re-prompts the model cold. Model answers from stale context (memory) instead of retrying the calendar call.

**Key existing machinery to mirror (read these first):**

| What | Where |
|---|---|
| `PendingApprovalResume` struct | `crates/ironclaw_agent_loop/src/state.rs:87-91` |
| Block-arm population (approval only) | `crates/ironclaw_agent_loop/src/executor/gates.rs:64-77` |
| Resume detection in prompt stage | `crates/ironclaw_agent_loop/src/executor/prompt.rs:239-249` |
| Candidate rebuild helper | `crates/ironclaw_agent_loop/src/executor/capability_helpers.rs:40-51` (`pending_approval_resume_candidate`) |
| Resume consumption in capability batch | `crates/ironclaw_agent_loop/src/executor/capabilities.rs:131-157` (`take_if` + `CapabilityApprovalResume`) |
| Clear-on-completion | `crates/ironclaw_agent_loop/src/executor/capabilities.rs:747-757` (`clear_matching_pending_approval_resume`) |
| Auth outcome → GateStage | `crates/ironclaw_agent_loop/src/executor/capabilities.rs:450-468` |
| `CapabilityCallCandidate` fields | `crates/ironclaw_turns/src/run_profile/host.rs:1211-1219` (`surface_version`, `capability_id`, `input_ref`, `effective_capability_ids`, `provider_replay`) |
| `PromptStep::ResumeApproval` routing | `crates/ironclaw_agent_loop/src/executor/canonical.rs` (grep `ResumeApproval`) |
| Test scaffolding | `crates/ironclaw_agent_loop/src/test_support/mod.rs:449,1063` (`ScriptedCapabilityOutcome::AuthRequired`) |

**Scope guard:** No changes to `ironclaw_reborn`, `ironclaw_product_workflow`, `ironclaw_loop_support`, or host-runtime crates. The candidate already carries everything needed for re-dispatch — this is purely executor/state work in `ironclaw_agent_loop` (plus, only if Task 1 proves it necessary, a serialization shim in `state/slots.rs`).

---

### Task 0: Investigate checkpoint serialization compatibility

The state must survive checkpoint round-trips (`LoopExecutionState::from_checkpoint_payload`, used by `PlannedDriver::resume`). Determine the serialization mechanism before touching state.

**Files:**
- Read: `crates/ironclaw_agent_loop/src/state.rs` (struct + `from_checkpoint_payload` / `to_checkpoint_payload`)
- Read: `crates/ironclaw_agent_loop/src/state/slots.rs` (note: contains manual code mappings like `indexed_message_kind_code` — the codec may be hand-rolled, not pure serde)

- [ ] **Step 1: Determine how `pending_approval_resume` is encoded into the checkpoint payload**

Run: `grep -n "pending_approval_resume\|checkpoint_payload\|serde" crates/ironclaw_agent_loop/src/state.rs crates/ironclaw_agent_loop/src/state/slots.rs | head -40`

Two possible outcomes:
- **Serde-based:** adding a new `#[serde(default)] Option<…>` field is backward compatible (old checkpoints deserialize with `None`). No schema version bump needed — confirm by finding an existing `#[serde(default)]` field that was added after v1.
- **Manual codec:** mirror exactly how `PendingApprovalResume` is encoded/decoded in `slots.rs`, with a length/presence guard so old payloads (missing the slot) decode to `None`. If the codec is strictly positional with no presence guard, bump `CHECKPOINT_SCHEMA_VERSION` in `crates/ironclaw_reborn/src/planned_driver.rs` (grep `CHECKPOINT_SCHEMA_VERSION`) — note this invalidates in-flight blocked runs across deploy, which is acceptable for local-dev but call it out in the PR description.

- [ ] **Step 2: Record the finding as a comment in your working notes and pick the corresponding branch in Task 1 Step 3**

No commit — investigation only.

---

### Task 1: Add `PendingAuthResume` state slot

**Files:**
- Modify: `crates/ironclaw_agent_loop/src/state.rs` (struct at ~line 87, `initial_for_run` defaults at ~line 138)
- Modify (only if manual codec): `crates/ironclaw_agent_loop/src/state/slots.rs`

- [ ] **Step 1: Write the failing round-trip test** (in the existing `#[cfg(test)]` module of `state.rs`, mirroring the existing checkpoint round-trip tests there — grep `from_checkpoint_payload` in the test module for the analog)

```rust
#[test]
fn pending_auth_resume_round_trips_through_checkpoint_payload() {
    let mut state = LoopExecutionState::initial_for_run(&test_run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:auth:test").expect("gate ref"),
        capability_id: CapabilityId::new("gsuite.calendar.list_events").expect("cap id"),
        surface_version: CapabilitySurfaceVersion::new("surface-v1").expect("surface"),
        input_ref: CapabilityInputRef::new("input:test").expect("input ref"),
        effective_capability_ids: vec![],
        provider_replay: None,
    });
    let payload = state.to_checkpoint_payload(CheckpointKind::BeforeBlock).expect("encode");
    let restored = LoopExecutionState::from_checkpoint_payload(payload.as_bytes(), CheckpointKind::BeforeBlock)
        .expect("decode");
    assert_eq!(restored.pending_auth_resume, state.pending_auth_resume);
}

#[test]
fn checkpoint_payload_without_auth_resume_slot_decodes_to_none() {
    // Encode a state that has no pending_auth_resume; decode must yield None
    // (guards backward compat with pre-existing checkpoints).
    let state = LoopExecutionState::initial_for_run(&test_run_context());
    let payload = state.to_checkpoint_payload(CheckpointKind::BeforeBlock).expect("encode");
    let restored = LoopExecutionState::from_checkpoint_payload(payload.as_bytes(), CheckpointKind::BeforeBlock)
        .expect("decode");
    assert!(restored.pending_auth_resume.is_none());
}
```

Adjust constructor names (`test_run_context`, `to_checkpoint_payload` signature, `CheckpointKind` variant) to match the existing round-trip tests in that module — copy their setup verbatim.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p ironclaw_agent_loop pending_auth_resume_round_trips -- --nocapture`
Expected: FAIL — `pending_auth_resume` field / `PendingAuthResume` type not found.

- [ ] **Step 3: Add the type and field**

In `state.rs`, next to `PendingApprovalResume` (~line 91):

```rust
/// Auth-gated capability call parked at a `BlockedAuth` checkpoint.
///
/// Unlike [`PendingApprovalResume`] there is no resume token: on resume the
/// call is re-dispatched as a fresh invocation and the host re-evaluates the
/// auth requirement (credentials now present → executes; still missing →
/// blocks again).
#[derive(Debug, Clone, PartialEq)]
pub struct PendingAuthResume {
    pub gate_ref: LoopGateRef,
    pub capability_id: CapabilityId,
    pub surface_version: CapabilitySurfaceVersion,
    pub input_ref: CapabilityInputRef,
    pub effective_capability_ids: Vec<CapabilityId>,
    pub provider_replay: Option<ProviderToolCallReplay>,
}
```

(Match derive list, field visibility, and import style of `PendingApprovalResume` exactly — including `Serialize`/`Deserialize` derives if that struct has them.)

On `LoopExecutionState`:

```rust
    pub pending_approval_resume: Option<PendingApprovalResume>,
    pub pending_auth_resume: Option<PendingAuthResume>,
```

If serde-based codec: add `#[serde(default, skip_serializing_if = "Option::is_none")]` on the new field. If manual codec: replicate the `PendingApprovalResume` encode/decode in `slots.rs` with a presence guard per Task 0's finding.

Initialize `pending_auth_resume: None` in `initial_for_run` (~line 138).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ironclaw_agent_loop pending_auth_resume -- --nocapture` and `cargo test -p ironclaw_agent_loop checkpoint`
Expected: PASS (both new tests, no regressions in existing checkpoint tests).

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_agent_loop/src/state.rs crates/ironclaw_agent_loop/src/state/slots.rs
git commit -m "feat(agent_loop): add PendingAuthResume checkpoint state slot"
```

---

### Task 2: Populate the slot in GateStage's Block arm for auth gates

**Files:**
- Modify: `crates/ironclaw_agent_loop/src/executor/gates.rs:61-101` (Block arm)
- Test: `crates/ironclaw_agent_loop/src/executor/tests.rs`

- [ ] **Step 1: Write the failing test.** Locate the existing approval-gate-block test as the template:

Run: `grep -n "pending_approval_resume\|ApprovalRequired" crates/ironclaw_agent_loop/src/executor/tests.rs | head -20`

Copy that test's harness setup, switch the scripted outcome to `ScriptedCapabilityOutcome::AuthRequired { gate_ref }` (`test_support/mod.rs:449`), run the loop to the blocked exit, and assert on the checkpointed state:

```rust
#[tokio::test]
async fn auth_gate_block_stores_pending_auth_resume_in_checkpoint() {
    // Harness setup copied from the approval-block analog test, with the
    // capability scripted to return AuthRequired.
    // ... (analog setup) ...
    let exit = /* run loop */;
    let LoopExit::Blocked(blocked) = exit else { panic!("expected blocked exit") };
    assert_eq!(blocked.kind, LoopBlockedKind::Auth);
    let state = /* decode checkpoint payload written by the harness, same as analog test */;
    let resume = state.pending_auth_resume.expect("auth resume record stored");
    assert_eq!(resume.gate_ref, blocked.gate_ref);
    assert_eq!(resume.capability_id.as_str(), "cap.test"); // whatever id the harness scripts
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p ironclaw_agent_loop auth_gate_block_stores -- --nocapture`
Expected: FAIL — `pending_auth_resume` is `None`.

- [ ] **Step 3: Populate in the Block arm.** In `gates.rs` Block arm, after the existing `state.pending_approval_resume = …` assignment (line 64-77), add:

```rust
                state.pending_auth_resume = match kind {
                    GateKind::Auth => Some(crate::state::PendingAuthResume {
                        gate_ref: gate_ref.clone(),
                        capability_id: call.capability_id.clone(),
                        surface_version: call.surface_version.clone(),
                        input_ref: call.input_ref.clone(),
                        effective_capability_ids: call.effective_capability_ids.clone(),
                        provider_replay: call.provider_replay.clone(),
                    }),
                    _ => state.pending_auth_resume.take(),
                };
```

(`call` is the `CapabilityCallCandidate` already in scope; all six fields exist on it plus `gate_ref` — no `GateInput` changes needed.)

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p ironclaw_agent_loop --lib executor`
Expected: PASS, including pre-existing gate tests.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_agent_loop/src/executor/gates.rs crates/ironclaw_agent_loop/src/executor/tests.rs
git commit -m "feat(agent_loop): store pending auth resume record when auth gate blocks"
```

---

### Task 3: Re-dispatch on resume in the prompt stage

**Files:**
- Modify: `crates/ironclaw_agent_loop/src/executor/prompt.rs` (~line 239, after the approval-resume check)
- Modify: `crates/ironclaw_agent_loop/src/executor/capability_helpers.rs` (new candidate helper next to `pending_approval_resume_candidate` at line 40)
- Modify: `crates/ironclaw_agent_loop/src/executor/canonical.rs` + wherever `PromptStep` is defined (grep `enum PromptStep`) — route the new variant identically to `ResumeApproval`
- Modify: `crates/ironclaw_agent_loop/src/executor/capabilities.rs` — clear slot on completion
- Test: `crates/ironclaw_agent_loop/src/executor/tests.rs`

- [ ] **Step 1: Write the failing end-to-end resume test.** Locate the approval-resume analog:

Run: `grep -n "ResumeApproval\|resume" crates/ironclaw_agent_loop/src/executor/tests.rs | head -20`

Test shape: block on `AuthRequired`, capture the checkpoint, build a fresh executor run from that checkpoint state (the analog test shows how), script the capability to now return `Completed`, and assert (a) the capability host received a re-invocation with the original `input_ref` and **no** approval resume attached, (b) the completed result is appended, (c) `pending_auth_resume` is cleared afterward.

```rust
#[tokio::test]
async fn resume_after_auth_gate_redispatches_original_call_without_model_turn() {
    // Phase 1: run until AuthRequired blocks; decode checkpoint state (as in Task 2 test).
    // Phase 2: re-enter executor with the decoded state; capability now scripted Completed.
    // ... (analog setup) ...
    // (a) host saw the re-dispatch with the original input_ref, resume token absent
    // (b) result appended to result_refs / capability batch summary
    // (c) state.pending_auth_resume is None after completion
    // (d) the model stage ran AFTER the capability completed (payload visible), not before
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p ironclaw_agent_loop resume_after_auth_gate -- --nocapture`
Expected: FAIL — no re-dispatch happens; loop goes straight to a model turn.

- [ ] **Step 3: Add the candidate helper** in `capability_helpers.rs` next to line 40:

```rust
pub(super) fn pending_auth_resume_candidate(
    resume: &PendingAuthResume,
    surface_version: CapabilitySurfaceVersion,
) -> CapabilityCallCandidate {
    CapabilityCallCandidate {
        surface_version,
        capability_id: resume.capability_id.clone(),
        input_ref: resume.input_ref.clone(),
        effective_capability_ids: resume.effective_capability_ids.clone(),
        provider_replay: resume.provider_replay.clone(),
    }
}
```

- [ ] **Step 4: Detect in prompt stage.** In `prompt.rs`, immediately after the `pending_approval_resume` check (line 239-249), add the parallel check (approval keeps priority — both set simultaneously is impossible today, but order it defensively):

```rust
        if let Some(resume) = self.state.pending_auth_resume.as_ref() {
            let call = pending_auth_resume_candidate(resume, surface.version.clone());
            return Ok(PromptStep::ResumeAuth(Box::new(ApprovalResumePromptOutput {
                state: self.state,
                pending_input_ack: self.pending_input_ack,
                surface,
                call,
            })));
        }
```

Add `PromptStep::ResumeAuth(Box<ApprovalResumePromptOutput>)` to the `PromptStep` enum and route it in `canonical.rs` through the **same** match-arm body as `ResumeApproval` (the downstream capability stage distinguishes them by which state slot is populated — `take_if` on `pending_approval_resume` at `capabilities.rs:139` simply finds nothing for an auth resume, so the invocation goes out without a resume token, which is exactly what we want).

- [ ] **Step 5: Clear on completion.** In `capabilities.rs`, mirror `clear_matching_pending_approval_resume` (line 747-757):

```rust
fn clear_matching_pending_auth_resume(state: &mut LoopExecutionState, call: &CapabilityCallCandidate) {
    if state
        .pending_auth_resume
        .as_ref()
        .is_some_and(|resume| resume.capability_id == call.capability_id)
    {
        state.pending_auth_resume = None;
    }
}
```

Call it at every site where `clear_matching_pending_approval_resume` is called (lines 204, 221, 243, 401, 418, 567 — grep to confirm, the file will have shifted). Also clear it in `handle_capability_error` paths so a failing re-dispatch doesn't loop forever — find where the approval slot is cleared on error and mirror.

- [ ] **Step 6: Run the full test**

Run: `cargo test -p ironclaw_agent_loop resume_after_auth_gate -- --nocapture`
Expected: PASS.

- [ ] **Step 7: Run full crate tests**

Run: `cargo test -p ironclaw_agent_loop`
Expected: PASS — especially existing `ResumeApproval` tests unchanged.

- [ ] **Step 8: Commit**

```bash
git add crates/ironclaw_agent_loop/src/executor/ crates/ironclaw_agent_loop/src/state.rs
git commit -m "feat(agent_loop): re-dispatch auth-gated capability call on resume"
```

---

### Task 4: Re-block idempotency + cross-crate verification

**Files:**
- Test: `crates/ironclaw_agent_loop/src/executor/tests.rs`
- Verify only (no edits): `crates/ironclaw_reborn/tests/loop_driver_host.rs`, `crates/ironclaw_turns/tests/agent_loop_host_contract.rs`

- [ ] **Step 1: Write the still-blocked test** — resume where the capability STILL returns `AuthRequired` (credentials not actually stored). Assert: loop exits `Blocked` again with a (possibly new) auth gate_ref, `pending_auth_resume` re-populated, no model turn consumed, no infinite loop.

```rust
#[tokio::test]
async fn resume_with_still_missing_credentials_blocks_again_without_model_turn() {
    // Same two-phase shape as Task 3 test, but phase-2 capability scripted
    // AuthRequired again. Expect LoopExit::Blocked(kind=Auth) and a stored
    // pending_auth_resume; model invocation count must be 0 in phase 2.
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p ironclaw_agent_loop resume_with_still_missing -- --nocapture`
Expected: PASS already if Tasks 2-3 are correct (Block arm re-stores). If it fails, fix the Block arm interaction before proceeding.

- [ ] **Step 3: Full quality gate**

Run: `cargo fmt && cargo clippy --all --benches --tests --examples --all-features && cargo test -p ironclaw_agent_loop -p ironclaw_reborn -p ironclaw_turns`
Expected: zero clippy warnings, all green. (`ironclaw_reborn`/`ironclaw_turns` consume `LoopExecutionState` and the driver contract — they must compile and pass untouched.)

- [ ] **Step 4: Commit**

```bash
git add -A crates/ironclaw_agent_loop
git commit -m "test(agent_loop): cover repeated auth block on resume"
```

---

## Out of scope (explicitly)

- Rendering a model-visible "auth completed" notice from `LoopInput::GateResolved` — superseded by re-dispatch.
- Any change to `ironclaw_product_workflow` auth continuation, OAuth gate providers, or WebUI handlers — the resume entry path already works.
- Approval-gate behavior — must be byte-for-byte unchanged (existing tests are the guard).

## Risks

1. **Checkpoint codec is manual/positional** → Task 0 decides; worst case bump `CHECKPOINT_SCHEMA_VERSION` (invalidates in-flight blocked runs; old checkpoints fall to `checkpoint_unavailable_exit`, which is the pre-existing degraded path).
2. **`ApprovalResumePromptOutput` naming** now serves two variants — acceptable; rename only if clippy/review demands (keep diff minimal).
3. **Double-slot conflict** (both approval+auth set): impossible today (one gate per block), but prompt stage checks approval first — deterministic either way.
