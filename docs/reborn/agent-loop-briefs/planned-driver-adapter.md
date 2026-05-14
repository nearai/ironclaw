# WS-7 — Planned Driver Adapter

**Workstream:** WS-7
**Crate touched:** `ironclaw_reborn`
**Depends on:** WS-6 (`AgentLoopExecutor` + `CanonicalAgentLoopExecutor`)
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §3, §14 (driver disambiguation glossary entry)

---

## 1. Scope

Bridge the framework crate (`ironclaw_agent_loop`) to the runner-facing `AgentLoopDriver` trait (`ironclaw_turns`). One small struct + one trait impl in `ironclaw_reborn`.

- `PlannedDriver` struct — **non-generic**. Holds `Arc<LoopFamily>` (opaque to this crate; produced by WS-3.5's registry) and `Arc<CanonicalAgentLoopExecutor>`. No `<P, E>` type parameters.
- `impl AgentLoopDriver for PlannedDriver` — wires `run` and `resume` through to the executor.
- Sanitized error mapping from `AgentLoopExecutorError` to `AgentLoopDriverError`.
- Driver descriptor produced from the registry/profile `LoopDriverId`; the checkpoint payload separately records the family's `LoopFamilyId` and `ComponentIdentity` for resume compatibility (the framework's reserved checkpoint schema is `CHECKPOINT_SCHEMA_ID` from WS-0).
- Constructor `PlannedDriver::from_family(driver_id, family, executor)` — the canonical path. `TurnRunner` resolves a family from `Arc<LoopFamilyRegistry>` (WS-3.5) then constructs the driver under the registry/profile driver id. No direct planner injection exists.

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
    canonical_executor::CanonicalAgentLoopExecutor,
    executor::{AgentLoopExecutor, AgentLoopExecutorError, HostStage},
    family::{LoopFamily, LoopFamilyId, LoopFamilyRegistry},
    state::{CHECKPOINT_SCHEMA_ID, LoopExecutionState},
};
use ironclaw_turns::{
    LoopExit, RunProfileVersion,
    run_profile::{
        AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverHost,
        AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, LoopDriverId,
    },
};

/// Adapter that turns a framework `LoopFamily` + canonical executor into an
/// `AgentLoopDriver` the `TurnRunnerWorker` can register and call.
///
/// **Non-generic.** The framework offers one canonical executor and exposes
/// loop families as opaque `Arc<LoopFamily>` values; there is no surface for
/// a downstream caller to inject a custom planner or executor. Test surface
/// (when needed) is "real `LoopFamily` from `LoopFamilyRegistry::with_families`
/// + `MockHost`", not "synthetic planner + `MockHost`". Strategy-level
/// granularity in tests lives inside `ironclaw_agent_loop` where strategies
/// are visible.
///
/// The framework crate (`ironclaw_agent_loop`) does not know about
/// `AgentLoopDriver`; this struct is the only bridge.
pub struct PlannedDriver {
    descriptor: AgentLoopDriverDescriptor,
    family: Arc<LoopFamily>,
    executor: Arc<CanonicalAgentLoopExecutor>,
}

impl PlannedDriver {
    /// Constructs a planned driver from a resolved `LoopFamily`. The
    /// descriptor is built from the family's `LoopFamilyId` + the framework's
    /// reserved checkpoint schema (`CHECKPOINT_SCHEMA_ID` from WS-0). The
    /// driver version is supplied by the caller and rolls only when the
    /// runner-side wire shape changes.
    ///
    /// Canonical construction path: `TurnRunner` resolves a family from
    /// `Arc<LoopFamilyRegistry>` (WS-3.5) and calls this constructor.
    pub fn from_family(
        driver_id: LoopDriverId,
        family: Arc<LoopFamily>,
        executor: Arc<CanonicalAgentLoopExecutor>,
        version: RunProfileVersion,
    ) -> Result<Self, AgentLoopDriverError> {
        // Driver id is a registry/profile identity, not the family id.
        // The default registration passes "reborn:planned-default"; the
        // checkpoint payload separately records LoopFamilyId + ComponentIdentity
        // for resume compatibility.
        let descriptor = AgentLoopDriverDescriptor::new(driver_id.as_str(), version)
            .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })?
            .with_checkpoint_schema(CHECKPOINT_SCHEMA_ID, version)
            .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })?;

        Ok(Self { descriptor, family, executor })
    }

    /// Convenience: resolve a family by id from the registry then construct
    /// the driver. Returns an `AgentLoopDriverError::InvalidRequest` if the
    /// id is unbound.
    pub fn from_registry(
        driver_id: LoopDriverId,
        registry: &LoopFamilyRegistry,
        id: &LoopFamilyId,
        executor: Arc<CanonicalAgentLoopExecutor>,
        version: RunProfileVersion,
    ) -> Result<Self, AgentLoopDriverError> {
        let family = registry.get(id).ok_or_else(|| AgentLoopDriverError::InvalidRequest {
            reason: format!("unknown loop family: {id}"),
        })?;
        Self::from_family(driver_id, family, executor, version)
    }
}

#[async_trait]
impl AgentLoopDriver for PlannedDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor { self.descriptor.clone() }

    async fn run(
        &self,
        request: AgentLoopDriverRunRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        validate_run_request(&request, &self.descriptor)?;
        let initial = LoopExecutionState::initial(&request.run_context);
        // The executor consumes `&LoopFamily` directly. The
        // `pub(crate) fn planner()` accessor on `LoopFamily` is invisible
        // outside `ironclaw_agent_loop`, so `PlannedDriver` cannot reach
        // into strategies — it can only hand the family to the executor.
        self.executor
            .execute_family(self.family.as_ref(), host, initial)
            .await
            .map_err(map_executor_error)
    }

    async fn resume(
        &self,
        request: AgentLoopDriverResumeRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        validate_resume_request(&request, &self.descriptor)?;
        // Use the canonical WS-10 load-side request/response shape — see
        // `checkpoint-store-and-resume.md` §3.1.
        let loaded = host
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id: request.checkpoint_id,
                expected_schema_id: request.resolved_run_profile.loop_driver
                    .checkpoint_schema_id.clone(),
                expected_schema_version: request.resolved_run_profile.loop_driver
                    .checkpoint_schema_version,
            })
            .await
            .map_err(|_| AgentLoopDriverError::Unavailable {
                reason: "checkpoint:unavailable".to_string(),
            })?;
        let resumed = LoopExecutionState::from_checkpoint_payload(
                loaded.payload.as_bytes(),
                loaded.kind,
            )
            .map_err(|e| AgentLoopDriverError::Failed {
                reason_kind: format!("checkpoint_rejected:{e}"),
            })?;
        self.executor
            .execute_family(self.family.as_ref(), host, resumed)
            .await
            .map_err(map_executor_error)
    }
}
```

The `CanonicalAgentLoopExecutor::execute_family(&LoopFamily, ...)` signature is owned by WS-6 (canonical executor). WS-6's `execute` was originally specified to take `&dyn AgentLoopPlanner`; with the LoopFamily-cluster amendments, the executor's public entry point becomes `execute_family(&LoopFamily, ...)`. Inside the framework crate, the executor uses `family.planner()` (crate-private) and `AgentLoopPlannerInternal` (crate-private) to consult strategies. WS-6's brief is updated to match.

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

If this brief includes registry wiring (recommended for end-to-end smoke testability), it adds a small helper used by app startup:

```rust
/// Builds a default planned driver from a resolved `LoopFamily` and the
/// canonical executor. Intended for registration in the driver registry
/// alongside the existing `TextOnlyModelReplyDriver`.
pub fn default_planned_driver(
    family_registry: &LoopFamilyRegistry,
    executor: Arc<CanonicalAgentLoopExecutor>,
) -> Result<PlannedDriver, AgentLoopDriverError> {
    PlannedDriver::from_registry(
        LoopDriverId::from_trusted_static("reborn:planned-default"),
        family_registry,
        &LoopFamilyId::DEFAULT,
        executor,
        RunProfileVersion::new(1),
    )
}
```

`LoopFamilyRegistry` is constructed once at app startup by
`ironclaw_reborn::app_loop_family::build_loop_family_registry()` (WS-3.5).
Registration in `driver_registry.rs` mirrors the existing pattern for
`TextOnlyModelReplyDriver`. This is optional for the skeleton — wiring lands
when there's a real use case (typically the first follow-up loop-family PR).

## 4. Acceptance criteria

- [ ] `cargo check -p ironclaw_reborn` passes
- [ ] `cargo clippy --all --benches --tests --examples --all-features` zero warnings
- [ ] Existing `TextOnlyModelReplyDriver` unchanged; its tests still pass: `cargo test -p ironclaw_reborn -- text_loop_driver`
- [ ] Trait conformance: `fn _check(_: &PlannedDriver) where PlannedDriver: AgentLoopDriver {}` (no generics)
- [ ] Round-trip test: `PlannedDriver::from_registry(LoopDriverId::from_trusted_static("reborn:planned-default"), &registry, &LoopFamilyId::DEFAULT, executor, v1)` succeeds against `LoopFamilyRegistry::with_families(vec![Arc::new(families::default())])`; descriptor's `id` is `"reborn:planned-default"`; descriptor's `checkpoint_schema_id` is `CHECKPOINT_SCHEMA_ID`; checkpoint metadata separately records `LoopFamilyId::DEFAULT`
- [ ] Resolution failure: `PlannedDriver::from_registry(LoopDriverId::from_trusted_static("reborn:planned-default"), &empty_registry, &LoopFamilyId("nope"), …)` returns `Err(InvalidRequest { reason: "unknown loop family: nope" })`
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
- `PlannedDriver` (non-generic) is the canonical adapter. Loop families
  are opaque `Arc<LoopFamily>` values resolved from `LoopFamilyRegistry`
  (WS-3.5); the driver wraps them. The framework crate has no knowledge
  of `AgentLoopDriver` — bridge logic lives only here. The strategy seal
  means downstream test code constructs real families from the registry,
  not synthetic planners.
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
