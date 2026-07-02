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

- [ ] **Step 2: Write the test**

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
    let script =
        ScenarioScript::same_calls_repeated("demo.echo", 4).with_capability_outcomes(vec![
            vec![ScriptedCapabilityOutcome::completed("result:repeat-1")],
            vec![ScriptedCapabilityOutcome::completed("result:repeat-2")],
            vec![ScriptedCapabilityOutcome::completed("result:repeat-3")],
            vec![ScriptedCapabilityOutcome::completed_no_change(
                "result:repeat-4",
            )],
        ]);
    let (host, _) = MockAgentLoopDriverHost::builder()
        .run_context(context)
        .script(script)
        .build();
    let request = run_request(&driver, &host);

    let exit = driver
        .run(request, &host)
        .await
        .expect("planned driver run should succeed");

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

This reuses the exact scripted no-progress scenario already proven to drive `NoProgressDetected` through the full strategy pipeline in `crates/ironclaw_agent_loop/tests/safety_nets.rs::repeated_signature_stops_after_rendered_warning_and_no_progress_result` (4 repeated `demo.echo` calls, the last reporting `completed_no_change`), but swaps in the real production-resolved `planned_default` profile instead of the default synthetic test context — so this is a "test through the caller" proof that the flag set in real profile resolution actually reaches and fires `try_final_answer_nudge`, not just that the struct field is `true` in isolation.

- [ ] **Step 3: Run the test**

Run: `cargo test -p ironclaw_reborn planned_default_profile_completes_via_final_answer_nudge`
Expected: PASS — Tasks 1–2 already enabled the flag, so this is a same-session confirmation test, not a red/green cycle. If it fails, check first whether the exit is `Failed(NoProgressDetected)` with `model_call_count() == 4` (nudge did not fire — re-check Task 2 landed) vs a compile error (import or helper usage mismatch).

- [ ] **Step 4: Commit**

```bash
git add crates/ironclaw_reborn/tests/planned_driver_e2e.rs
git commit -m "test(ironclaw_reborn): prove final-answer nudge fires for real planned_default profile"
```

---

### Task 7: Integration test — `scheduled_trigger` nudge fires end-to-end

**Files:**
- Modify: `crates/ironclaw_reborn/tests/planned_driver_e2e.rs`

**Interfaces:**
- Consumes: same as Task 6, plus `ironclaw_turns::RunProfileId::scheduled_trigger()` and `ironclaw_turns::RunProfileRequest::new(..)` (both fully-qualified, matching this file's existing style at line 210).

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
    let script =
        ScenarioScript::same_calls_repeated("demo.echo", 4).with_capability_outcomes(vec![
            vec![ScriptedCapabilityOutcome::completed("result:repeat-1")],
            vec![ScriptedCapabilityOutcome::completed("result:repeat-2")],
            vec![ScriptedCapabilityOutcome::completed("result:repeat-3")],
            vec![ScriptedCapabilityOutcome::completed_no_change(
                "result:repeat-4",
            )],
        ]);
    let (host, _) = MockAgentLoopDriverHost::builder()
        .run_context(context)
        .script(script)
        .build();
    let request = run_request(&driver, &host);

    let exit = driver
        .run(request, &host)
        .await
        .expect("planned driver run should succeed");

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

- [ ] **Step 2: Run the test**

Run: `cargo test -p ironclaw_reborn scheduled_trigger_profile_completes_via_final_answer_nudge`
Expected: PASS — same reasoning as Task 6 Step 3.

- [ ] **Step 3: Commit**

```bash
git add crates/ironclaw_reborn/tests/planned_driver_e2e.rs
git commit -m "test(ironclaw_reborn): prove final-answer nudge fires for real scheduled_trigger profile"
```

---

### Task 8: Full verification gate

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
