# WS-10 — Checkpoint Store + Resume Path

**Workstream:** WS-10 (follow-up; not in the skeleton WS-0..WS-8 set)
**Crates touched:** `ironclaw_turns` (trait extension) + `ironclaw_loop_support` + `ironclaw_reborn`
**Depends on:** WS-7 (`PlannedDriver` adapter), WS-8 (skeleton green)
**Parallel with:** WS-9, WS-11, WS-12, WS-13, WS-15
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §11–§12

---

## 1. Scope

Today the skeleton can *write* checkpoints — `CanonicalAgentLoopExecutor`
calls `host.checkpoint(LoopCheckpointRequest)` at the four boundary kinds
(`BeforeModel`, `BeforeSideEffect`, `BeforeBlock`, `Final`) via
`LoopCheckpointPort` ([`crates/ironclaw_turns/src/run_profile/host.rs:1108`](../../../crates/ironclaw_turns/src/run_profile/host.rs)).
The trait, however, has **no read method**, and `PlannedDriver::resume`
(WS-7) has nothing to fetch its initial state from. The canonical executor
contract in master-doc §8 opens with "`state = LoopExecutionState::initial(...)`
OR `::from_checkpoint` on resume" — but the second branch is unreachable
without a load path.

WS-10 closes that gap in three layers, in line with the §12 ownership rule:

1. **Trait extension** — add `LoopCheckpointPort::load_checkpoint_payload`
   in `ironclaw_turns`. Default impl returns `Err(Unavailable)` so the
   existing stubs keep compiling and tests stay green.
2. **Existing adapter gains the load method** — `HostManagedLoopCheckpointPort`
   at [`crates/ironclaw_reborn/src/loop_driver_host.rs:1499`](../../../crates/ironclaw_reborn/src/loop_driver_host.rs)
   already composes `CheckpointStateStore` (payload) + `LoopCheckpointStore`
   (metadata) and implements `checkpoint(...)`. WS-10 extends *that* impl
   with `load_checkpoint_payload(...)`. No new adapter is introduced.
   The brief explicitly does **not** redefine the write path — the
   staged-state-ref flow (the executor stages bytes upstream, hands the
   port a `LoopCheckpointStateRef`, the port validates + writes
   metadata) is owned by WS-0 / WS-6 and stays unchanged.
3. **Resume wiring** — `PlannedDriver::resume` calls the new method,
   deserializes via the `LoopExecutionState::from_checkpoint_payload`
   constructor reserved by WS-0
   ([`state-and-checkpoints.md`](state-and-checkpoints.md) §3),
   and passes the resulting state to
   `CanonicalAgentLoopExecutor::execute_family(family, host, initial_state)`
   (the same entry point used by the run path; resume only differs in how
   `initial_state` is sourced).

WS-10 does **not** pick a persistent backend. `CheckpointStateStore` is
already an abstract trait; concrete PostgreSQL / libSQL adapters live in
`src/db/` and land as separate PRs scoped against the storage layer.

## 2. Files

### NEW
_None._ WS-10 extends existing types rather than introducing a new
adapter. (Originally drafted with a new
`CheckpointStateStoreLoopCheckpointPort` in `ironclaw_loop_support`;
that adapter is redundant because `HostManagedLoopCheckpointPort`
already composes both stores.)

### MODIFIED
- `crates/ironclaw_turns/src/run_profile/host.rs` —
  `LoopCheckpointPort` gains `load_checkpoint_payload`. Default impl
  forwards through `unsupported_host_method("load_checkpoint_payload")`
  (the helper at `host.rs:1189`).
- `crates/ironclaw_turns/src/loop_exit.rs` —
  additive variant `LoopFailureKind::CheckpointUnavailable`
  (line 427 enum). `to_sanitized_failure` gains a matching arm
  (`"checkpoint_unavailable"`).
- `crates/ironclaw_reborn/src/text_loop_driver.rs` —
  `loop_failure_kind_name` matches `LoopFailureKind` exhaustively
  and needs a matching arm for `CheckpointUnavailable` (`"checkpoint_unavailable"`).
- `crates/ironclaw_reborn/src/milestone_events.rs` — the
  `loop_failure_kind` helper at
  [line 208](../../../crates/ironclaw_reborn/src/milestone_events.rs)
  is similarly exhaustive over `LoopFailureKind` and needs the same
  arm. Both name helpers must agree, since they both feed sanitized
  failure strings into the milestone/projection layers.
- `crates/ironclaw_reborn/src/loop_driver_host.rs` —
  `HostManagedLoopCheckpointPort` (line 1499) gains a
  `load_checkpoint_payload` impl that reads metadata via
  `loop_checkpoint_store.get_loop_checkpoint(...)` (resolves
  `state_ref`), validates schema id/version, then reads payload via
  `checkpoint_state_store.get_checkpoint_state(...)`. Reuses the
  existing `turn_error_to_host_error` helper already in the file.
- `crates/ironclaw_reborn/src/planned_driver.rs` (WS-7 file) —
  `resume(claim)` calls `host.load_checkpoint_payload(...)`, decodes
  via `LoopExecutionState::from_checkpoint_payload`, and forwards to
  the executor. The two store fields are already plumbed through
  `RebornLoopDriverHost`'s construction at `loop_driver_host.rs:1233`;
  this brief does not add new config fields.

### NOT TOUCHED
- `crates/ironclaw_turns/src/checkpoint_state.rs` — both stores already
  expose `get_*` reads (`get_checkpoint_state`, `get_loop_checkpoint`).
  This brief consumes that surface unchanged.
- The contract crate guardrails (`crates/ironclaw_turns/CLAUDE.md`) —
  storage adapter selection stays out of `ironclaw_turns`. The trait
  extension records lifecycle metadata only; the backing impl lives below.
- The staged-state-ref write flow. The executor (WS-6) stages payload
  bytes via `CheckpointStateStore::put_checkpoint_state` upstream and
  hands the resulting `LoopCheckpointStateRef` into the existing
  `LoopCheckpointRequest`. WS-10 reads but does not redefine that
  contract.

## 3. Specification

### 3.1 `LoopCheckpointPort::load_checkpoint_payload`

```rust
//! crates/ironclaw_turns/src/run_profile/host.rs (delta near line 1108)

#[async_trait]
pub trait LoopCheckpointPort: Send + Sync {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError>;

    /// Load the redacted payload behind a previously-written checkpoint.
    ///
    /// Resume callers (`PlannedDriver::resume`, recovery probes) MUST call
    /// this method, never reach below the port. Returns the bytes only —
    /// schema id / schema version are returned alongside so callers can
    /// reject payloads they cannot decode rather than blindly trusting
    /// the run's expected schema.
    ///
    /// Default impl reports unavailable so adapters that have not yet
    /// implemented the load path (e.g. test stubs, the skeleton
    /// `EmptyLoopCheckpointPort` style) keep compiling.
    async fn load_checkpoint_payload(
        &self,
        _request: LoadCheckpointPayloadRequest,
    ) -> Result<LoadedCheckpointPayload, AgentLoopHostError> {
        Err(unsupported_host_method("load_checkpoint_payload"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadCheckpointPayloadRequest {
    pub checkpoint_id: TurnCheckpointId,
    /// Expected schema id+version from `LoopRunContext`. The adapter
    /// rejects mismatches at the port boundary so executor code never
    /// has to defend against cross-version payloads.
    pub expected_schema_id: CheckpointSchemaId,
    pub expected_schema_version: RunProfileVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedCheckpointPayload {
    pub kind: LoopCheckpointKind,
    pub schema_id: CheckpointSchemaId,
    pub schema_version: RunProfileVersion,
    pub payload: RedactedCheckpointPayload,
}
```

Notes:

- `LoadedCheckpointPayload.payload` is `RedactedCheckpointPayload`
  ([`checkpoint_state.rs:20`](../../../crates/ironclaw_turns/src/checkpoint_state.rs))
  — opaque to callers above the executor; `LoopExecutionState::from_checkpoint_payload`
  is the only thing that decodes its bytes.
- The default impl deliberately uses `unsupported_host_method` so the
  blanket `impl AgentLoopDriverHost for T` at `host.rs:1170` keeps
  working for every existing mock and the runner-side fallback path
  surfaces a clean `AgentLoopHostErrorKind::Unavailable`.

### 3.2 `LoopFailureKind::CheckpointUnavailable`

```rust
//! crates/ironclaw_turns/src/loop_exit.rs (delta at line 427)

pub enum LoopFailureKind {
    ModelError,
    ContextBuildFailed,
    CapabilityProtocolError,
    IterationLimit,
    InvalidModelOutput,
    CheckpointRejected,
    /// The resume path could not load the checkpoint payload (store
    /// unavailable, record missing, schema id/version mismatch, payload
    /// fails redaction-aware deserialization). Distinct from
    /// `CheckpointRejected`, which fires when the executor decides at
    /// write time that a checkpoint must not be taken.
    CheckpointUnavailable,
    TranscriptWriteFailed,
    DriverBug,
    InterruptedUnexpectedly,
    NoProgressDetected,   // reserved by WS-0; included here for context
}
```

`to_sanitized_failure` gains a matching arm
(`"checkpoint_unavailable"`). The skeleton's WS-0 brief already
documents the `NoProgressDetected` follow-up addition; placing
`CheckpointUnavailable` next to `CheckpointRejected` keeps the two
checkpoint-related variants adjacent.

### 3.3 Extending `HostManagedLoopCheckpointPort`

The existing impl at
[`crates/ironclaw_reborn/src/loop_driver_host.rs:1529`](../../../crates/ironclaw_reborn/src/loop_driver_host.rs)
already composes both stores. WS-10's contribution is just the
`load_checkpoint_payload` body — the `checkpoint` body is unchanged.

```rust
//! crates/ironclaw_reborn/src/loop_driver_host.rs (delta near line 1529)

#[async_trait]
impl LoopCheckpointPort for HostManagedLoopCheckpointPort {
    // EXISTING — unchanged:
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        // ... existing body, lines 1530-1571 ...
    }

    // NEW (WS-10):
    async fn load_checkpoint_payload(
        &self,
        request: LoadCheckpointPayloadRequest,
    ) -> Result<LoadedCheckpointPayload, AgentLoopHostError> {
        // 1. Resolve loop-checkpoint metadata → state_ref + recorded schema.
        let meta = self
            .loop_checkpoint_store
            .get_loop_checkpoint(GetLoopCheckpointRequest {
                scope: self.run_context.scope.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                checkpoint_id: request.checkpoint_id,
            })
            .await
            .map_err(turn_error_to_host_error)?
            .ok_or_else(|| AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "checkpoint metadata not found".to_string(),
            ))?;

        // 2. Schema gate — reject before the larger payload read.
        if meta.schema_id != request.expected_schema_id
            || meta.schema_version != request.expected_schema_version
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Invalid,
                "checkpoint schema id/version mismatch".to_string(),
            ));
        }

        // 3. Pull the staged payload via the state_ref the metadata points at.
        let state_record = self
            .checkpoint_state_store
            .get_checkpoint_state(GetCheckpointStateRequest {
                scope: self.run_context.scope.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                state_ref: meta.state_ref,
                schema_id: meta.schema_id.clone(),
                schema_version: meta.schema_version,
                kind: meta.kind,
            })
            .await
            .map_err(turn_error_to_host_error)?
            .ok_or_else(|| AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "checkpoint payload not found for state_ref".to_string(),
            ))?;

        Ok(LoadedCheckpointPayload {
            kind: state_record.kind,
            schema_id: state_record.schema_id,
            schema_version: state_record.schema_version,
            payload: state_record.payload,
        })
    }
}
```

`turn_error_to_host_error` already exists in `loop_driver_host.rs`
(used by the `checkpoint` arm) and maps `TurnError::Unavailable` →
`AgentLoopHostErrorKind::Unavailable`, etc. No raw error details
cross the boundary; per `error-handling.md` channel-edge rule the
public `AgentLoopHostError` only carries a short safe summary.

**Crate-ownership note:** the impl extension lives in
`ironclaw_reborn`, not `ironclaw_loop_support`, because
`HostManagedLoopCheckpointPort` is already housed in
`crates/ironclaw_reborn/src/loop_driver_host.rs`. Moving the type to
`ironclaw_loop_support` is an orthogonal cleanup; the brief
deliberately does not bundle it.

### 3.4 Resume path in `PlannedDriver`

```rust
//! crates/ironclaw_reborn/src/planned_driver.rs (delta; WS-7 owns the rest)

impl AgentLoopDriver for PlannedDriver {
    async fn resume(
        &self,
        claim: ResumedRunClaim,
    ) -> Result<LoopExit, AgentLoopDriverError> {
        let host = self.build_host(&claim.run_context).await?;
        let payload = host
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id: claim.last_checkpoint_id,
                expected_schema_id: claim.run_context.checkpoint_schema_id.clone(),
                expected_schema_version: claim.run_context.checkpoint_schema_version,
            })
            .await
            .map_err(|e| AgentLoopDriverError::from_host(e))?;

        let initial = LoopExecutionState::from_checkpoint_payload(
            payload.payload.as_bytes(),
            payload.kind,
        ).map_err(|reason| AgentLoopDriverError::failed(
            LoopFailureKind::CheckpointUnavailable,
            reason,
        ))?;

        let exit = self.executor
            .execute_family(self.family.as_ref(), &host, initial)
            .await?;
        Ok(exit)
    }
}
```

Two notes:

- `ResumedRunClaim.last_checkpoint_id: TurnCheckpointId` comes from the
  runner-side claim (already populated when `TurnRunState` transitioned
  through `BeforeBlock` or `Final` — see `TurnRunState::CancelRequested`
  flow in `crates/ironclaw_turns/src/runner.rs:137`).
- `CanonicalAgentLoopExecutor::execute_family(family, host, initial_state)`
  is the single executor entry point per WS-6 (post-LoopFamily-amendment).
  Resume uses `LoopExecutionState::from_checkpoint_payload` to source
  `initial_state` and then calls the same `execute_family` method that
  fresh runs use; no separate executor method exists.

### 3.5 Failure semantics

| Failure                                       | Mapping                                                |
|-----------------------------------------------|--------------------------------------------------------|
| Metadata store returns `None`                 | `AgentLoopHostErrorKind::Unavailable` → `LoopFailureKind::CheckpointUnavailable` |
| Payload store returns `None`                  | same                                                   |
| Schema id mismatch                            | `AgentLoopHostErrorKind::Invalid` → `LoopFailureKind::CheckpointUnavailable` (still "we cannot resume from this") |
| `RedactedCheckpointPayload::new` rejects size | impossible at read (already validated at write); on the unlikely path, map through `Invalid` |
| `LoopExecutionState::from_checkpoint_payload` rejects | `LoopFailureKind::CheckpointUnavailable` with a short reason |

When `PlannedDriver::resume` cannot produce a viable initial state, it
returns `LoopExit::Failed { reason_kind: CheckpointUnavailable }`. The
runner records the failure normally; the active-run lock releases per
the existing two-phase cancellation flow (no extra wiring needed).

## 4. Composition in `PlannedDriverConfig`

No new fields. `RebornLoopDriverHost` (the host facade
`PlannedDriver` composes) already constructs
`HostManagedLoopCheckpointPort` from `Arc<dyn CheckpointStateStore>`
+ `Arc<dyn LoopCheckpointStore>` at
[`crates/ironclaw_reborn/src/loop_driver_host.rs:1233`](../../../crates/ironclaw_reborn/src/loop_driver_host.rs).
WS-10's contribution is just the `load_checkpoint_payload` method on
that existing impl — no new dependencies, no new wiring.

`PlannedDriver::resume(claim)` consumes the load method:

```rust
//! crates/ironclaw_reborn/src/planned_driver.rs (resume delta; WS-7 owns the rest)

impl AgentLoopDriver for PlannedDriver {
    async fn resume(
        &self,
        claim: ResumedRunClaim,
    ) -> Result<LoopExit, AgentLoopDriverError> {
        let host = self.build_host(&claim.run_context).await?;
        let payload = host
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id: claim.last_checkpoint_id,
                expected_schema_id: claim.run_context.checkpoint_schema_id.clone(),
                expected_schema_version: claim.run_context.checkpoint_schema_version,
            })
            .await
            .map_err(AgentLoopDriverError::from_host)?;

        let initial = LoopExecutionState::from_checkpoint_payload(
            payload.payload.as_bytes(),
            payload.kind,
        ).map_err(|reason| AgentLoopDriverError::failed(
            LoopFailureKind::CheckpointUnavailable,
            reason,
        ))?;

        self.executor
            .execute_family(self.family.as_ref(), &host, initial)
            .await
            .map_err(Into::into)
    }
}
```

## 5. Verification

Unit tests (in `crates/ironclaw_reborn`, alongside the existing
`HostManagedLoopCheckpointPort` tests):

- `loop_driver_host::tests::load_payload_roundtrip` — stage payload via `InMemoryCheckpointStateStore`, write metadata via `checkpoint(...)` → get `TurnCheckpointId`; call `load_checkpoint_payload(checkpoint_id)`; assert payload bytes equal.
- `loop_driver_host::tests::load_payload_rejects_schema_mismatch` — write under `reborn:default-loop-v1`, attempt load with `expected_schema_id = reborn:default-loop-v2`, assert `Invalid` error.
- `loop_driver_host::tests::load_payload_missing_metadata_is_unavailable` — request a `TurnCheckpointId` that was never written; assert `Unavailable`.
- `loop_driver_host::tests::load_payload_missing_state_record_is_unavailable` — write metadata, drop the state row from the store; assert `Unavailable`.
- `loop_driver_host::tests::load_payload_size_already_capped_at_write` — `MAX_CHECKPOINT_STATE_PAYLOAD_BYTES` ([`checkpoint_state.rs:13`](../../../crates/ironclaw_turns/src/checkpoint_state.rs)) is enforced by `RedactedCheckpointPayload::new` at stage time; the load path simply returns the stored payload. Regression guard asserts that.

Integration tests (in `crates/ironclaw_reborn`, gated behind
the existing `ironclaw_agent_loop/test-support` feature from WS-8):

- `planned_driver_resume_from_before_side_effect_replays_state` —
  drive one iteration, intercept at `BeforeSideEffect` checkpoint, drop
  the in-memory executor, reconstruct `PlannedDriver` with the same
  stores, call `resume(claim)`, assert the second run produces an
  exit that includes the same assistant refs / result refs as a
  full-run baseline.
- `planned_driver_resume_unknown_checkpoint_id_fails_cleanly` — point
  `resume` at a bogus `TurnCheckpointId`; assert
  `LoopExit::Failed { reason_kind: CheckpointUnavailable }`.
- `planned_driver_resume_schema_drift_fails_cleanly` — bump the run
  context's `checkpoint_schema_version` between write and resume;
  assert `CheckpointUnavailable` again.

## 6. Out of scope (for this brief)

- **Concrete persistent backend** — PostgreSQL / libSQL impls of
  `CheckpointStateStore` and `LoopCheckpointStore`. Lands as a
  separate PR scoped against `src/db/`. The `InMemory*` impls in
  `crates/ironclaw_turns/src/checkpoint_state.rs` are sufficient for
  WS-10's verification.
- **GC / retention** — pruning old checkpoints after a run terminates.
  Backend concern; not visible at the port.
- **Cross-version replay** — strict equality on
  `(schema_id, schema_version)` in v1. Schema migration is a separate
  design problem and lives outside the WS-0..WS-15 set.
- **Checkpoint compression / chunking** — the 64 KB cap in
  `MAX_CHECKPOINT_STATE_PAYLOAD_BYTES` is enough for `DefaultPlanner`
  state (signature ring of 8 + failure ring of 8 + minor slots). Larger
  planners can revisit when needed.
- **Multi-region replication** — backend concern.
