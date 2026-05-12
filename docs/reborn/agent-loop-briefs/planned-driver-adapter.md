# WS-7 — Planned Driver Adapter

**Workstream:** WS-7
**Crate touched:** `ironclaw_reborn`
**Depends on:** WS-6 (`AgentLoopExecutor` + `CanonicalAgentLoopExecutor`)
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §3, §14 (driver disambiguation glossary entry)

---

## 1. Scope

Bridge the framework crate (`ironclaw_agent_loop`) to the runner-facing `AgentLoopDriver` trait (`ironclaw_turns`). One small struct + one trait impl in `ironclaw_reborn`.

- `PlannedDriver<P, E>` struct — generic over planner and executor.
- `impl AgentLoopDriver for PlannedDriver<P, E>` — wires `run` and `resume` through to the executor.
- Sanitized error mapping from `AgentLoopExecutorError` to `AgentLoopDriverError`.
- Driver descriptor produced from the planner's `PlannerId` (with a stable version policy).
- Optional: a registry-side helper that constructs a `PlannedDriver<DefaultPlanner, CanonicalAgentLoopExecutor>` and registers it under a known `LoopDriverId` for end-to-end smoke tests once the framework is exercised.

## 2. Files

### NEW
- `crates/ironclaw_reborn/src/planned_driver.rs` — struct, impl, error mapping
- `crates/ironclaw_reborn/CLAUDE.md` — crate guardrail (see §6 below). Today this crate has no top-level CLAUDE.md; WS-7 introduces one alongside `PlannedDriver` since this is the first non-trivial integration code landing here under the new framework.

### EXTEND (only if registry wiring is included)
- `crates/ironclaw_reborn/src/driver_registry.rs` — register the planned driver under its descriptor

### NOT TOUCHED
- `crates/ironclaw_reborn/src/text_loop_driver.rs` — `TextOnlyModelReplyDriver` stays exactly as-is
- `crates/ironclaw_reborn/src/turn_runner.rs` — no surface change; new drivers register through the existing registry
- `ironclaw_agent_loop` — this brief reads from it but doesn't extend it

## 3. Specification

### 3.1 `PlannedDriver`

```rust
//! crates/ironclaw_reborn/src/planned_driver.rs

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_agent_loop::{
    executor::{AgentLoopExecutor, AgentLoopExecutorError, HostStage},
    planner::AgentLoopPlanner,
    state::{CHECKPOINT_SCHEMA_ID, LoopExecutionState},
};
use ironclaw_turns::{
    LoopExit, RunProfileVersion,
    run_profile::{
        AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverHost,
        AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest,
    },
};

/// Adapter that turns a framework planner + executor into an
/// `AgentLoopDriver` the `TurnRunnerWorker` can register and call.
///
/// The framework crate (`ironclaw_agent_loop`) does not know about
/// `AgentLoopDriver`; this struct is the only bridge.
///
/// `P` is typically `DefaultPlanner` (or a loop-family planner that wraps
/// it). `E` is typically `CanonicalAgentLoopExecutor`.
pub struct PlannedDriver<P: AgentLoopPlanner, E: AgentLoopExecutor> {
    descriptor: AgentLoopDriverDescriptor,
    planner: Arc<P>,
    executor: Arc<E>,
}

impl<P: AgentLoopPlanner, E: AgentLoopExecutor> PlannedDriver<P, E> {
    /// Constructs a planned driver. The descriptor is built from the planner's
    /// PlannerId with the supplied version + the framework's reserved
    /// checkpoint schema id (CHECKPOINT_SCHEMA_ID from WS-0).
    pub fn new(
        planner: Arc<P>,
        executor: Arc<E>,
        version: RunProfileVersion,
    ) -> Result<Self, AgentLoopDriverError> {
        let descriptor = AgentLoopDriverDescriptor::new(
            planner.id().as_str(),
            version,
        )
        .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })?
        .with_checkpoint_schema(CHECKPOINT_SCHEMA_ID, version)
        .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })?;

        Ok(Self { descriptor, planner, executor })
    }
}

#[async_trait]
impl<P, E> AgentLoopDriver for PlannedDriver<P, E>
where
    P: AgentLoopPlanner + 'static,
    E: AgentLoopExecutor + 'static,
{
    fn descriptor(&self) -> AgentLoopDriverDescriptor { self.descriptor.clone() }

    async fn run(
        &self,
        request: AgentLoopDriverRunRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        validate_run_request(&request, &self.descriptor)?;
        let initial = LoopExecutionState::initial();
        self.executor
            .execute(self.planner.as_ref(), host, initial)
            .await
            .map_err(map_executor_error)
    }

    async fn resume(
        &self,
        request: AgentLoopDriverResumeRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        validate_resume_request(&request, &self.descriptor)?;
        let payload = host
            .load_checkpoint_payload(/* request.checkpoint_id */)
            .await
            .map_err(|_| AgentLoopDriverError::Unavailable {
                reason: "checkpoint:unavailable".to_string(),
            })?;
        let resumed = LoopExecutionState::from_checkpoint_payload(&payload)
            .map_err(|e| AgentLoopDriverError::Failed {
                reason_kind: format!("checkpoint_rejected:{e}"),
            })?;
        self.executor
            .execute(self.planner.as_ref(), host, resumed)
            .await
            .map_err(map_executor_error)
    }
}
```

### 3.2 Request validation

`PlannedDriver` only validates **descriptor assignment** — the narrow check that "this driver is the one the run profile selected." The broader checks that turn/run IDs and resolved profile match the host's run context belong to **`TurnRunner`** (it claimed the run; it owns context-match assertions). Splitting the validation cleanly:

```rust
// PlannedDriver-side: descriptor-only check
fn validate_descriptor_assignment(
    request_profile: &ResolvedRunProfile,
    descriptor: &AgentLoopDriverDescriptor,
) -> Result<(), AgentLoopDriverError> {
    if request_profile.loop_driver != *descriptor {
        return Err(AgentLoopDriverError::InvalidRequest {
            reason: "driver request profile is not assigned to this planned driver".to_string(),
        });
    }
    Ok(())
}

fn validate_run_request(
    request: &AgentLoopDriverRunRequest,
    descriptor: &AgentLoopDriverDescriptor,
) -> Result<(), AgentLoopDriverError> {
    validate_descriptor_assignment(&request.resolved_run_profile, descriptor)
}

fn validate_resume_request(
    request: &AgentLoopDriverResumeRequest,
    descriptor: &AgentLoopDriverDescriptor,
) -> Result<(), AgentLoopDriverError> {
    validate_descriptor_assignment(&request.resolved_run_profile, descriptor)?;
    // Schema-id check: ensure the checkpoint we're being asked to resume from
    // matches the schema this descriptor was constructed with. Mismatched
    // schema = framework version drift; reject as InvalidRequest so the runner
    // can route the run to a recovery path instead of resuming with stale data.
    //
    // `AgentLoopDriverResumeRequest` does NOT carry a checkpoint_schema_id
    // field directly (today its fields are turn_id, run_id, checkpoint_id,
    // resolved_run_profile). The schema id lives on the resolved profile's
    // loop_driver descriptor — that's what the runner pinned at submit time
    // and what the checkpoint payload was tagged with.
    let want = descriptor.checkpoint_schema_id.as_ref();
    let have = request.resolved_run_profile.loop_driver.checkpoint_schema_id.as_ref();
    if want != have {
        return Err(AgentLoopDriverError::InvalidRequest {
            reason: "checkpoint schema id does not match driver descriptor".to_string(),
        });
    }
    Ok(())
}
```

**Out of scope for `PlannedDriver`** (these stay in `TurnRunner` / `LoopExitApplier`):
- `request.turn_id == host.run_context().turn_id` and `request.run_id == host.run_context().run_id`
- `request.resolved_run_profile == host.run_context().resolved_run_profile`

The runner already validates context-match before invoking any driver — duplicating it inside `PlannedDriver` (as the existing `TextOnlyModelReplyDriver` does today) is a code-smell carry-over from a pre-`PlannedDriver` world. WS-7 takes the opportunity to fix the boundary.

### 3.3 Error mapping

```rust
fn map_executor_error(err: AgentLoopExecutorError) -> AgentLoopDriverError {
    tracing::warn!(error = ?err, "planned driver executor returned sanitized error");
    match err {
        AgentLoopExecutorError::HostUnavailable { stage } => {
            AgentLoopDriverError::Unavailable { reason: format!("{stage:?}: unavailable") }
        }
        AgentLoopExecutorError::PlannerContract { detail } => {
            AgentLoopDriverError::Failed { reason_kind: format!("driver_bug:{detail}") }
        }
        AgentLoopExecutorError::CheckpointFailed { stage } => {
            AgentLoopDriverError::Failed { reason_kind: format!("checkpoint_rejected:{stage:?}") }
        }
        AgentLoopExecutorError::Cancelled => {
            // Clean cancellation surfaces as `Ok(LoopExit::Cancelled(...))` from
            // the executor (see WS-6 §3.5). This branch ONLY fires for the
            // unrecoverable edge case where the executor could not even produce
            // a `LoopExit::Cancelled` (e.g. the cancellation checkpoint write
            // itself failed). Map to Failed { interrupted_unexpectedly } so the
            // runner records a terminal failure with a clear category — NOT
            // Unavailable, which would mis-signal a transient infrastructure
            // problem.
            AgentLoopDriverError::Failed {
                reason_kind: "interrupted_unexpectedly".to_string(),
            }
        }
    }
}
```

The doc comment must call out that `AgentLoopDriverError` strings never carry raw provider errors, host paths, secrets, or tool input — the executor sanitizes upstream (per `error-handling.md` channel-edge rule).

### 3.4 Optional registry wiring

If this brief includes registry wiring (recommended for end-to-end smoke testability), it adds a small constructor used by app startup:

```rust
/// Builds a default planned driver: DefaultPlanner + CanonicalAgentLoopExecutor.
/// Intended for registration in the driver registry alongside the existing
/// TextOnlyModelReplyDriver.
pub fn default_planned_driver() -> Result<PlannedDriver<
    ironclaw_agent_loop::default_planner::DefaultPlanner,
    ironclaw_agent_loop::canonical_executor::CanonicalAgentLoopExecutor,
>, AgentLoopDriverError> {
    let planner = Arc::new(ironclaw_agent_loop::default_planner::DefaultPlanner::default());
    let executor = Arc::new(ironclaw_agent_loop::canonical_executor::CanonicalAgentLoopExecutor::default());
    PlannedDriver::new(planner, executor, RunProfileVersion::new(1))
}
```

Registration in `driver_registry.rs` mirrors the existing pattern for `TextOnlyModelReplyDriver`. This is optional for the skeleton — wiring lands when there's a real use case (typically the first follow-up loop-family PR).

## 4. Acceptance criteria

- [ ] `cargo check -p ironclaw_reborn` passes
- [ ] `cargo clippy --all --benches --tests --examples --all-features` zero warnings
- [ ] Existing `TextOnlyModelReplyDriver` unchanged; its tests still pass: `cargo test -p ironclaw_reborn -- text_loop_driver`
- [ ] Trait conformance: `fn _check<P: AgentLoopPlanner + 'static, E: AgentLoopExecutor + 'static>(_: &PlannedDriver<P, E>) where PlannedDriver<P, E>: AgentLoopDriver {}`
- [ ] Round-trip test: `PlannedDriver::new(DefaultPlanner::default(), CanonicalAgentLoopExecutor::default(), v1)` succeeds; descriptor's `id` is `"reborn:default-loop"`; descriptor's `checkpoint_schema_id` is `CHECKPOINT_SCHEMA_ID`
- [ ] Error-mapping tests:
  - `map_executor_error(HostUnavailable { stage: Model })` → `Unavailable { reason: "Model: unavailable" }`
  - `map_executor_error(CheckpointFailed { stage: BeforeModel })` → `Failed { reason_kind: "checkpoint_rejected:BeforeModel" }`
  - mapped `AgentLoopDriverError` debug output contains no raw provider names, no `/` paths, no secret-shaped strings (mirror the existing `text_loop_driver` test pattern)
- [ ] Smoke test using a `MockAgentLoopDriverHost` that returns a Reply on first call:
  - `PlannedDriver::run(req, &host)` returns `LoopExit::Completed` with assistant ref
  - host recorder shows the four-checkpoint sequence (`BeforeModel`, `Final`)
- [ ] Resume smoke test: load a checkpoint payload produced by serializing `LoopExecutionState`; assert `from_checkpoint_payload` accepts it; assert mismatched schema id is rejected with `Failed { reason_kind: "checkpoint_rejected:..." }`

## 5. Out of scope

- A real `LoopCapabilityPort` (still `EmptyLoopCapabilityPort` until a tool-capable driver lands)
- Registry wiring — optional; recommended but not required for the skeleton
- Migration of `TextOnlyModelReplyDriver` to a `TextOnlyPlanner` factory — explicitly deferred per master doc §11
- `ModelRouteChain` migration of `LoopRunContext.resolved_model_route` — deferred per master doc §9

## 6. Crate guardrail (`crates/ironclaw_reborn/CLAUDE.md`)

Suggested content:

```markdown
# ironclaw_reborn guardrails

- Owns runtime integration for the agent loop: driver registration, executor wiring,
  exit validation, run-profile resolution. Bridges the runner-facing
  `AgentLoopDriver` trait (defined in `ironclaw_turns`) to the framework
  (`ironclaw_agent_loop` planner + executor).
- Depends on `ironclaw_agent_loop` for planner + executor; depends on
  `ironclaw_turns` for the `AgentLoopDriver` trait + descriptor + `LoopExit`.
  Does NOT re-export framework types — consumers import them from
  `ironclaw_agent_loop` directly.
- `PlannedDriver<P, E>` is the canonical adapter. Loop families register concrete
  `PlannedDriver` instances under stable `LoopDriverId`s in `DriverRegistry`.
  The framework crate has no knowledge of `AgentLoopDriver` — bridge logic
  lives only here.
- Request validation in `PlannedDriver` is **descriptor-assignment only**.
  Turn/run ID matching and resolved-profile matching belong to `TurnRunner`,
  not the driver adapter. Do not duplicate runner-level checks here.
- Existing `TextOnlyModelReplyDriver` stays untouched until a tool-capable
  driver follow-up justifies migration to a `TextOnlyPlanner` factory.
- `LoopExitApplier` (existing) validates evidence in returned `LoopExit` values
  and applies durable transitions. Driver impls return `LoopExit`; they never
  call `TurnRunner` transition APIs directly.
- Master spec: `docs/reborn/agent-loop-skeleton.md`. Brief that introduced this
  crate's framework integration: `docs/reborn/agent-loop-briefs/planned-driver-adapter.md`.
```

## 7. Verification command sequence

```bash
cargo check -p ironclaw_reborn
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test -p ironclaw_reborn
cargo test -p ironclaw_agent_loop  # ensure nothing in framework crate broke from the integration
```

End-to-end agent-loop verification (an actual run through the `TurnRunnerWorker` invoking a `PlannedDriver`) requires a working `LoopCapabilityPort` impl and is the property of the first follow-up loop-family PR.
