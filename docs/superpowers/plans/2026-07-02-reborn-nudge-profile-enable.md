# Enable Reborn Final-Answer Nudges for planned_default and scheduled_trigger Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn on Reborn's final-answer nudge (`SteeringPolicy.allow_driver_specific_nudges`) for the `planned_default` (real interactive/chat/CLI traffic) and `scheduled_trigger` run profiles, while `subagent` stays off, and prove it fires through the real profile-resolution + driver path, not just as a struct field.

**Architecture:** Add one builder method, `RunProfileDefinition::with_driver_specific_nudges(bool)`, mirroring the existing `with_personal_context_policy` pattern. Chain it at the two profile-definition functions that should opt in (`planned_default_profile_definition`, `scheduled_trigger_planned_profile_definition`); leave the shared helper they both call, and the `subagent` sibling, untouched.

**Tech Stack:** Rust, `ironclaw_turns` (neutral run-profile contracts), `ironclaw_reborn` (planned-driver factory), `ironclaw_agent_loop` (loop executor + test support).

## Global Constraints

- `cargo fmt` must leave no diff.
- `cargo clippy --all --benches --tests --examples --all-features` must be clean (zero warnings).
- `cargo test` (unit tier, no external deps) must pass for every touched crate.
- `.unwrap()`/`.expect()` are fine in test code; do not add them to production code (none of these tasks touch production code that isn't test-covered).
- Crate boundaries: `ironclaw_turns` must not depend on `ironclaw_reborn`; `ironclaw_agent_loop` must not depend on `ironclaw_reborn`. The integration test in Tasks 6–7 belongs in `crates/ironclaw_reborn/tests/` (which already depends on both), never in `crates/ironclaw_agent_loop/tests/`.
- Every fix/behavior change gets a regression test per this repo's testing discipline (`.claude/rules/testing.md`) — write/extend the test before the behavior change, watch it fail for the right reason, then change the code.
- Commit after each task (frequent, atomic commits).

---

### Task 1: Add `with_driver_specific_nudges` builder method on `RunProfileDefinition`

**Files:**
- Modify: `crates/ironclaw_turns/src/run_profile/resolver.rs:226` (add method right after `with_personal_context_policy`)
- Test: `crates/ironclaw_turns/src/run_profile/resolver.rs` (inside `mod tests`, e.g. right after `direct_authority_preserves_allowed`, currently ending at line 632)

**Interfaces:**
- Produces: `RunProfileDefinition::with_driver_specific_nudges(self, enabled: bool) -> Self` — a builder method later tasks chain onto `interactive_profile()`-derived definitions.

- [ ] **Step 1: Write the failing test**

Add this test inside `mod tests` in `crates/ironclaw_turns/src/run_profile/resolver.rs` (the module already has `use super::*;` at its top, so `interactive_profile()` and `RunProfileResolutionRequest` are in scope):

```rust
    #[test]
    fn driver_specific_nudges_builder_toggles_steering_policy() {
        let default_profile = interactive_profile();
        assert!(!default_profile.steering_policy.allow_driver_specific_nudges);

        let nudged = interactive_profile().with_driver_specific_nudges(true);
        let snapshot = nudged.resolve(&RunProfileResolutionRequest::interactive_default());
        assert!(snapshot.steering_policy.allow_driver_specific_nudges);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw_turns driver_specific_nudges_builder_toggles_steering_policy`
Expected: FAIL to compile — `no method named 'with_driver_specific_nudges' found for struct 'RunProfileDefinition'`.

- [ ] **Step 3: Write the builder method**

In `crates/ironclaw_turns/src/run_profile/resolver.rs`, immediately after `with_personal_context_policy` (currently lines 226–229):

```rust
    pub fn with_personal_context_policy(mut self, policy: PersonalContextPolicy) -> Self {
        self.personal_context_policy = policy;
        self
    }

    pub fn with_driver_specific_nudges(mut self, enabled: bool) -> Self {
        self.steering_policy.allow_driver_specific_nudges = enabled;
        self
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ironclaw_turns driver_specific_nudges_builder_toggles_steering_policy`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_turns/src/run_profile/resolver.rs
git commit -m "feat(ironclaw_turns): add RunProfileDefinition::with_driver_specific_nudges builder"
```

---

### Task 2: Enable driver-specific nudges for `planned_default`

**Files:**
- Modify: `crates/ironclaw_reborn/src/planned_driver_factory.rs:249-260` (`planned_default_profile_definition`)
- Test: `crates/ironclaw_reborn/src/planned_driver_factory.rs:422-450` (`profile_resolves_to_planned_driver`)

**Interfaces:**
- Consumes: `RunProfileDefinition::with_driver_specific_nudges(bool) -> Self` (Task 1).

- [ ] **Step 1: Extend the failing test**

In `crates/ironclaw_reborn/src/planned_driver_factory.rs`, add an assertion to the end of `profile_resolves_to_planned_driver` (currently ending at line 450):

```rust
        assert_eq!(
            snapshot
                .loop_driver
                .checkpoint_schema_id
                .as_ref()
                .map(|id| id.as_str()),
            Some(PLANNED_DRIVER_CHECKPOINT_SCHEMA_ID)
        );
        assert!(
            snapshot.steering_policy.allow_driver_specific_nudges,
            "planned_default must have driver-specific nudges enabled"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw_reborn profile_resolves_to_planned_driver`
Expected: FAIL — `assertion failed: snapshot.steering_policy.allow_driver_specific_nudges`.

- [ ] **Step 3: Enable the flag**

In `crates/ironclaw_reborn/src/planned_driver_factory.rs`, change `planned_default_profile_definition` (lines 249–260) from:

```rust
pub fn planned_default_profile_definition() -> Result<RunProfileDefinition, RunProfileRegistryError>
{
    let descriptor = planned_driver_descriptor()
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    let profile_id = planned_default_profile_id()
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    planned_like_profile_definition(
        profile_id,
        descriptor,
        INTERACTIVE_CAPABILITY_SURFACE_PROFILE_ID,
    )
}
```

to:

```rust
pub fn planned_default_profile_definition() -> Result<RunProfileDefinition, RunProfileRegistryError>
{
    let descriptor = planned_driver_descriptor()
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    let profile_id = planned_default_profile_id()
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    planned_like_profile_definition(
        profile_id,
        descriptor,
        INTERACTIVE_CAPABILITY_SURFACE_PROFILE_ID,
    )
    .map(|definition| definition.with_driver_specific_nudges(true))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ironclaw_reborn profile_resolves_to_planned_driver`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_reborn/src/planned_driver_factory.rs
git commit -m "feat(ironclaw_reborn): enable driver-specific nudges for planned_default profile"
```

---

### Task 3: Enable driver-specific nudges for `scheduled_trigger`

**Files:**
- Modify: `crates/ironclaw_reborn/src/planned_driver_factory.rs:279-288` (`scheduled_trigger_planned_profile_definition`)
- Test: `crates/ironclaw_reborn/src/planned_driver_factory.rs:474-501` (`scheduled_trigger_profile_resolves_with_denied_surface_id`)

**Interfaces:**
- Consumes: `RunProfileDefinition::with_driver_specific_nudges(bool) -> Self` (Task 1).

- [ ] **Step 1: Extend the failing test**

Add an assertion to the end of `scheduled_trigger_profile_resolves_with_denied_surface_id`:

```rust
        assert_eq!(
            snapshot.capability_surface_profile_id.as_str(),
            SCHEDULED_TRIGGER_CAPABILITY_SURFACE_PROFILE_ID
        );
        assert!(
            snapshot.steering_policy.allow_driver_specific_nudges,
            "scheduled_trigger must have driver-specific nudges enabled"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw_reborn scheduled_trigger_profile_resolves_with_denied_surface_id`
Expected: FAIL — `assertion failed: snapshot.steering_policy.allow_driver_specific_nudges`.

- [ ] **Step 3: Enable the flag**

Change `scheduled_trigger_planned_profile_definition` (lines 279–288) from:

```rust
pub fn scheduled_trigger_planned_profile_definition()
-> Result<RunProfileDefinition, RunProfileRegistryError> {
    let descriptor = planned_driver_descriptor()
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    planned_like_profile_definition(
        RunProfileId::scheduled_trigger(),
        descriptor,
        SCHEDULED_TRIGGER_CAPABILITY_SURFACE_PROFILE_ID,
    )
}
```

to:

```rust
pub fn scheduled_trigger_planned_profile_definition()
-> Result<RunProfileDefinition, RunProfileRegistryError> {
    let descriptor = planned_driver_descriptor()
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    planned_like_profile_definition(
        RunProfileId::scheduled_trigger(),
        descriptor,
        SCHEDULED_TRIGGER_CAPABILITY_SURFACE_PROFILE_ID,
    )
    .map(|definition| definition.with_driver_specific_nudges(true))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ironclaw_reborn scheduled_trigger_profile_resolves_with_denied_surface_id`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_reborn/src/planned_driver_factory.rs
git commit -m "feat(ironclaw_reborn): enable driver-specific nudges for scheduled_trigger profile"
```

---

### Task 4: Regression guard — `subagent` stays off

**Files:**
- Modify: `crates/ironclaw_reborn/src/planned_driver_factory.rs:452-472` (`subagent_profile_resolves_to_subagent_planned_driver`)

**Interfaces:**
- None (test-only change; no production code touched by this task).

- [ ] **Step 1: Add the guard assertion**

Add to the end of `subagent_profile_resolves_to_subagent_planned_driver`:

```rust
        assert_eq!(
            snapshot.capability_surface_profile_id.as_str(),
            SUBAGENT_CAPABILITY_SURFACE_PROFILE_ID
        );
        assert!(
            !snapshot.steering_policy.allow_driver_specific_nudges,
            "subagent must not have driver-specific nudges enabled"
        );
    }
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p ironclaw_reborn subagent_profile_resolves_to_subagent_planned_driver`
Expected: PASS immediately — Tasks 1–3 never touched `subagent_planned_profile_definition`, so this is a regression guard, not a red/green cycle. If it fails, Task 2 or 3 leaked the flag into the shared `planned_like_profile_definition` helper instead of opting in per-call-site — stop and fix that before continuing.

- [ ] **Step 3: Commit**

```bash
git add crates/ironclaw_reborn/src/planned_driver_factory.rs
git commit -m "test(ironclaw_reborn): guard subagent profile against driver-specific nudges"
```

---

### Task 5: Reword the stale "off in production" doc comment

**Files:**
- Modify: `crates/ironclaw_agent_loop/src/executor/loop_exit.rs:32-34`

**Interfaces:**
- None (doc comment only).

- [ ] **Step 1: Update the comment**

Change:

```rust
/// Gated by `SteeringPolicy.allow_driver_specific_nudges` (off in production) and
/// capped at one nudge per run. Returns `Ok(None)` when disabled, capped, or the
/// model still declines to answer — callers then keep their existing behavior.
```

to:

```rust
/// Gated by `SteeringPolicy.allow_driver_specific_nudges` (enabled for select
/// Reborn run profiles — see `ironclaw_reborn::planned_driver_factory`; off by
/// default elsewhere) and capped at one nudge per run. Returns `Ok(None)` when
/// disabled, capped, or the model still declines to answer — callers then keep
/// their existing behavior.
```

- [ ] **Step 2: Run fmt and build to confirm no breakage**

Run: `cargo fmt -p ironclaw_agent_loop && cargo build -p ironclaw_agent_loop`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add crates/ironclaw_agent_loop/src/executor/loop_exit.rs
git commit -m "docs(ironclaw_agent_loop): correct stale off-in-production nudge comment"
```

---

### Task 6: Integration test — `planned_default` nudge fires end-to-end

**Files:**
- Modify: `crates/ironclaw_reborn/tests/planned_driver_e2e.rs`

**Interfaces:**
- Consumes: `default_planned_run_profile_resolver()` (already imported in this file, from `ironclaw_reborn::planned_driver_factory`), `PlannedDriver::default_from_registry`, `MockAgentLoopDriverHost::builder()`, `ScenarioScript::same_calls_repeated`, `ScriptedCapabilityOutcome::{completed, completed_no_change}` (new import needed), the existing `run_request` helper (line 39) and `LoopRunContext::new` pattern already used by `planned_driver_live_default_smoke` (line 143).
- Produces: `fn no_progress_script() -> ScenarioScript` and `fn assert_completed_via_nudge(exit: LoopExit, host: &MockAgentLoopDriverHost)` — Task 7 reuses both instead of duplicating the scenario/assertion.

- [ ] **Step 1: Add the missing import**

In `crates/ironclaw_reborn/tests/planned_driver_e2e.rs`, change the `ironclaw_agent_loop` import block (lines 7–13) from:

```rust
use ironclaw_agent_loop::{
    state::CheckpointKind,
    test_support::{
        MockAgentLoopDriverHost, MockHostCall, ScenarioScript, ScriptedModelResponse,
        test_run_context,
    },
};
```

to:

```rust
use ironclaw_agent_loop::{
    state::CheckpointKind,
    test_support::{
        MockAgentLoopDriverHost, MockHostCall, ScenarioScript, ScriptedCapabilityOutcome,
        ScriptedModelResponse, test_run_context,
    },
};
```

- [ ] **Step 2: Add two shared test helpers**

Task 7 needs the identical no-progress script and completion assertion. Rather than copy-pasting ~15 lines into each of the two new tests (this file already extracts repeated setup into helpers — see `run_request`, `resume_request`, `run_context_for_driver` near the top), add these two small free functions once, right after the existing `run_context_for_driver` helper (which currently ends at line 92):

```rust
/// Scripted no-progress scenario: 4 repeated identical calls, the last
/// reporting no change — proven to drive `NoProgressDetected` through the
/// full strategy pipeline (mirrors
/// `safety_nets.rs::repeated_signature_stops_after_rendered_warning_and_no_progress_result`).
fn no_progress_script() -> ScenarioScript {
    ScenarioScript::same_calls_repeated("demo.echo", 4).with_capability_outcomes(vec![
        vec![ScriptedCapabilityOutcome::completed("result:repeat-1")],
        vec![ScriptedCapabilityOutcome::completed("result:repeat-2")],
        vec![ScriptedCapabilityOutcome::completed("result:repeat-3")],
        vec![ScriptedCapabilityOutcome::completed_no_change(
            "result:repeat-4",
        )],
    ])
}

/// Asserts a no-progress run resolved via the final-answer nudge: one extra
/// tool-free model call on top of the 4 scripted repeats, and a completed
/// (not failed) exit.
fn assert_completed_via_nudge(exit: LoopExit, host: &MockAgentLoopDriverHost) {
    assert!(
        matches!(exit, LoopExit::Completed(_)),
        "no-progress exit should complete via the final-answer nudge, got {exit:?}"
    );
    assert_eq!(
        host.model_call_count(),
        5,
        "4 repeated calls + 1 tool-free nudge call"
    );
}
```

- [ ] **Step 3: Write the test**

Add this test to the bottom of the `#[tokio::test]` section of `crates/ironclaw_reborn/tests/planned_driver_e2e.rs` (e.g. right after `planned_driver_live_default_smoke`, which ends at line 175):

```rust
#[tokio::test]
async fn planned_default_profile_completes_via_final_answer_nudge() {
    let resolver = default_planned_run_profile_resolver().expect("resolver should build");
    let resolved = resolver
        .resolve_run_profile(ironclaw_turns::RunProfileResolutionRequest::interactive_default())
        .await
        .expect("planned_default profile should resolve");
    assert_eq!(resolved.profile_id.as_str(), PLANNED_DEFAULT_PROFILE_ID);
    assert!(
        resolved.steering_policy.allow_driver_specific_nudges,
        "planned_default must have driver-specific nudges enabled"
    );

    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let base_context = test_run_context("planned-default-nudge");
    let context = LoopRunContext::new(
        base_context.scope,
        base_context.turn_id,
        base_context.run_id,
        resolved.clone(),
    );
    let (host, _) = MockAgentLoopDriverHost::builder()
        .run_context(context)
        .script(no_progress_script())
        .build();
    let request = run_request(&driver, &host);

    let exit = driver
        .run(request, &host)
        .await
        .expect("planned driver run should succeed");

    assert_completed_via_nudge(exit, &host);
}
```

This reuses the exact scripted no-progress scenario already proven to drive `NoProgressDetected` through the full strategy pipeline in `crates/ironclaw_agent_loop/tests/safety_nets.rs::repeated_signature_stops_after_rendered_warning_and_no_progress_result` (4 repeated `demo.echo` calls, the last reporting `completed_no_change`), but swaps in the real production-resolved `planned_default` profile instead of the default synthetic test context — so this is a "test through the caller" proof that the flag set in real profile resolution actually reaches and fires `try_final_answer_nudge`, not just that the struct field is `true` in isolation.

- [ ] **Step 4: Run the test**

Run: `cargo test -p ironclaw_reborn planned_default_profile_completes_via_final_answer_nudge`
Expected: PASS — Tasks 1–2 already enabled the flag, so this is a same-session confirmation test, not a red/green cycle. If it fails, check first whether the exit is `Failed(NoProgressDetected)` with `model_call_count() == 4` (nudge did not fire — re-check Task 2 landed) vs a compile error (import or helper usage mismatch).

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_reborn/tests/planned_driver_e2e.rs
git commit -m "test(ironclaw_reborn): prove final-answer nudge fires for real planned_default profile"
```

---

### Task 7: Integration test — `scheduled_trigger` nudge fires end-to-end

**Files:**
- Modify: `crates/ironclaw_reborn/tests/planned_driver_e2e.rs`

**Interfaces:**
- Consumes: same as Task 6, including the `no_progress_script()` / `assert_completed_via_nudge(..)` helpers added in Task 6 Step 2, plus `ironclaw_turns::RunProfileId::scheduled_trigger()` and `ironclaw_turns::RunProfileRequest::new(..)` (both fully-qualified, matching this file's existing style at line 210).

- [ ] **Step 1: Write the test**

Add this test directly after the one added in Task 6:

```rust
#[tokio::test]
async fn scheduled_trigger_profile_completes_via_final_answer_nudge() {
    let resolver = default_planned_run_profile_resolver().expect("resolver should build");
    let resolved = resolver
        .resolve_run_profile(
            ironclaw_turns::RunProfileResolutionRequest::interactive_default()
                .with_requested_run_profile(
                    ironclaw_turns::RunProfileRequest::new(
                        ironclaw_turns::RunProfileId::scheduled_trigger().as_str(),
                    )
                    .unwrap(),
                ),
        )
        .await
        .expect("scheduled_trigger profile should resolve");
    assert_eq!(
        resolved.profile_id.as_str(),
        ironclaw_turns::RunProfileId::scheduled_trigger().as_str()
    );
    assert!(
        resolved.steering_policy.allow_driver_specific_nudges,
        "scheduled_trigger must have driver-specific nudges enabled"
    );

    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let base_context = test_run_context("scheduled-trigger-nudge");
    let context = LoopRunContext::new(
        base_context.scope,
        base_context.turn_id,
        base_context.run_id,
        resolved.clone(),
    );
    let (host, _) = MockAgentLoopDriverHost::builder()
        .run_context(context)
        .script(no_progress_script())
        .build();
    let request = run_request(&driver, &host);

    let exit = driver
        .run(request, &host)
        .await
        .expect("planned driver run should succeed");

    assert_completed_via_nudge(exit, &host);
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p ironclaw_reborn scheduled_trigger_profile_completes_via_final_answer_nudge`
Expected: PASS — same reasoning as Task 6 Step 4.

- [ ] **Step 3: Commit**

```bash
git add crates/ironclaw_reborn/tests/planned_driver_e2e.rs
git commit -m "test(ironclaw_reborn): prove final-answer nudge fires for real scheduled_trigger profile"
```

---

### Task 9: Flat product-level integration test — nudge fires through real `submit_turn`

**Added mid-execution** (user request, after Tasks 1–8 were planned): Tasks 6–7
prove the nudge fires at the `PlannedDriver`/executor tier with a real
resolved profile. This task proves it one layer up — through the actual
production `submit_turn` entry point (product workflow → turn coordinator →
scheduler → agent loop → real `LlmProviderModelGateway` decorator chain →
scripted model), using `RebornIntegrationHarness` per
`tests/support/reborn/CLAUDE.md`. Not a `reborn_group_*` test: the group
harness (e.g. `tests/reborn_group_extensions/`) exists for scenarios needing
**multiple threads over shared state** (its own doc comment: "an extension
installed by thread A is visible to thread B because both share the same
underlying store"). This scenario is single-thread, single-turn — per
`tests/support/reborn/CLAUDE.md` ("A scenario that submits + asserts in one
thread belongs in a flat `tests/reborn_integration_*.rs` test as always"),
the correct analog is a flat test, not a group.

**Feasibility, checked before writing this task:**
- `RebornIntegrationHarness` has no run-profile override — `submit_turn` always
  goes through the real `SubmitTurnRequest { requested_run_profile: None, .. }`
  path (`crates/ironclaw_reborn_composition/src/runtime.rs:2064`), which
  resolves to `planned_default` by default — so a plain
  `RebornIntegrationHarness::test_default()` already exercises the profile
  Task 2 enabled nudges for, with no special wiring.
- `CapabilityProgress::NoChange` (the signal the no-progress/repetition
  detector needs) is computed generically in production code from real
  capability output, not something only test mocks can fabricate:
  `crates/ironclaw_agent_loop/src/executor/capabilities.rs:1288-1309` compares
  the output digest of a call against previously-seen digests for the same
  call signature — a second identical `builtin.echo` call naturally produces
  `NoChange`, exactly mirroring Task 6/7's driver-tier scripted
  `ScriptedCapabilityOutcome::completed_no_change`.
- `builtin.echo` takes `{"message": "..."}` (confirmed via
  `tests/reborn_qa_routines.rs:463` and `tests/e2e/mock_llm.py:149`) and is
  enabled via `.with_builtin_http_tools()` on the harness builder.

**Files:**
- Create: `tests/reborn_integration_nudge_final_answer.rs`

**Interfaces:**
- Consumes: `RebornIntegrationHarness::test_default()`, `.with_builtin_http_tools()`,
  `RebornScriptedReply::{tool_call, text}`, `.submit_turn(text)`,
  `.assert_reply_contains(text)` — all from `tests/support/reborn/` per its
  `CLAUDE.md` (`build → submit_turn → assert` shape, include boilerplate from
  `tests/reborn_integration_greeting.rs`).

- [ ] **Step 1: Write the test — starting shape**

Create `tests/reborn_integration_nudge_final_answer.rs`. Copy the two
mandatory `#[path]`/`mod` include lines from `tests/reborn_integration_greeting.rs`
(per `tests/support/reborn/CLAUDE.md`) at the top, then:

```rust
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;

#[tokio::test]
async fn no_progress_repeated_echo_completes_via_final_answer_nudge() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.echo", serde_json::json!({"message": "same"})),
            RebornScriptedReply::tool_call("builtin.echo", serde_json::json!({"message": "same"})),
            RebornScriptedReply::tool_call("builtin.echo", serde_json::json!({"message": "same"})),
            RebornScriptedReply::tool_call("builtin.echo", serde_json::json!({"message": "same"})),
            RebornScriptedReply::text("final answer synthesized via nudge"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("say the same thing four times").await.expect("turn completes");
    h.assert_reply_contains("final answer synthesized via nudge").await.expect("reply finalized");
}
```

This mirrors Task 6/7's driver-tier script (4 identical calls to trigger the
repetition/no-progress detector, then a 5th scripted reply for the nudge's
tool-free call) at the product-level tier. **This starting shape is a
best-effort based on the feasibility check above, not verified by running
`cargo test`** — unlike every other task in this plan. If the run behaves
differently than expected (e.g. the repetition detector needs a different
call count, the harness needs an additional scripted turn, or the assertion
needs adjusting), iterate the same way Task 6 did: read the actual test
output, trace it against `crates/ironclaw_agent_loop/src/strategies/stop.rs`'s
`DefaultStopConditionStrategy` (repetition_threshold=3, repetition_window=5,
defaults at stop.rs:254-266) and `crates/ironclaw_agent_loop/src/executor/loop_exit.rs`'s
`try_final_answer_nudge`, and adjust the script/assertions to match reality.
Document any deviation in the report the same way Task 6 did.

- [ ] **Step 2: Run the test, iterate until it passes for the right reason**

Run: `cargo test --test reborn_integration_nudge_final_answer`

If it fails, do not guess blindly — read the failure (does the reply not
contain the marker? does the turn not complete? does `submit_turn` itself
error?) and adjust. If after reasonable iteration the scenario cannot be
made to pass without a production-code change (which would be out of scope —
this task should only need test-side adjustments, since Tasks 1–7 already
prove the production mechanism works), report BLOCKED with the specifics
rather than guessing further.

- [ ] **Step 3: Commit**

```bash
git add tests/reborn_integration_nudge_final_answer.rs
git commit -m "test(reborn): prove final-answer nudge fires through real submit_turn path"
```

---

### Task 8: Full verification gate

**Note:** run this task's steps twice — once after Task 7 (already done), and
once more after Task 9 lands, since Task 9 adds a new test file that also
needs to pass fmt/clippy/the full suite.

**Files:** none (verification only).

- [ ] **Step 1: Format**

Run: `cargo fmt`
Expected: no diff (`git diff --stat` empty after).

- [ ] **Step 2: Lint**

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: zero warnings.

- [ ] **Step 3: Unit tests, touched crates**

Run: `cargo test -p ironclaw_turns -p ironclaw_reborn -p ironclaw_agent_loop`
Expected: all pass, including the 6 tests added/extended in Tasks 1–4, 6, 7.

- [ ] **Step 4: Full workspace unit tests**

Run: `cargo test`
Expected: all pass (confirms nothing else in the workspace snapshot-asserted the old `false` value for these two profiles).

- [ ] **Step 5: Integration tier**

Run: `cargo test --features integration`
Expected: all pass (or skip cleanly if PostgreSQL is unreachable, per this repo's convention).

- [ ] **Step 6: If everything is green and nothing to fix, this task has no commit** — Tasks 1–7 already committed their changes individually.
