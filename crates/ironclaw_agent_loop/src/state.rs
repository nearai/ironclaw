//! Immutable loop execution state.
//!
//! See `docs/reborn/agent-loop-skeleton.md` sections 5-7 for the mutability
//! model and `docs/reborn/agent-loop-briefs/state-and-checkpoints.md` for this
//! crate foundation.

mod bounded_ring;
mod signature;
mod slots;

pub use bounded_ring::BoundedRing;
pub use ironclaw_turns::LoopFailureKind;
pub use signature::{ArgsHash, CapabilityCallSignature};
pub use slots::{
    CapabilityStrategyState, ContextStrategyState, ControlStrategyState, ModelStrategyState,
    RecoveryStrategyState,
};

use ironclaw_turns::{
    LoopGateRef, LoopMessageRef, LoopResultRef,
    run_profile::{CapabilitySurfaceVersion, LoopInputCursor, LoopRunContext},
};

/// Checkpoint payload schema reserved for the default Reborn loop.
pub const CHECKPOINT_SCHEMA_ID: &str = "reborn:default-loop-v1";

/// Immutable execution state threaded through the loop.
///
/// The executor rebinds its local `let mut state` each tick to the next whole
/// state. Strategies receive `&LoopExecutionState` and return outcome enums
/// that carry the new value of their own slot. The executor builds the next
/// whole state by swapping that slot.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LoopExecutionState {
    pub iteration: u32,
    pub last_checkpoint: Option<CheckpointMarker>,
    pub assistant_refs: Vec<LoopMessageRef>,
    pub result_refs: Vec<LoopResultRef>,
    pub last_gate: Option<LoopGateRef>,
    pub input_cursor: LoopInputCursor,
    pub surface_version: Option<CapabilitySurfaceVersion>,
    pub recent_call_signatures: BoundedRing<CapabilityCallSignature, 8>,
    pub recent_failure_kinds: BoundedRing<LoopFailureKind, 8>,
    pub context_state: ContextStrategyState,
    pub capability_state: CapabilityStrategyState,
    pub model_state: ModelStrategyState,
    pub recovery_state: RecoveryStrategyState,
    pub control_state: ControlStrategyState,
    /// Wall-clock anchor for `BudgetStrategy::wall_clock_limit` enforcement.
    ///
    /// Iter-6 finding 1: persisted so the run's effective start time
    /// survives `Blocked`, process restart, or checkpoint reload — the
    /// pre-iter-6 executor anchored each `execute()` entry to a fresh
    /// `tokio::time::Instant`, which is monotonic-only and resets across
    /// restarts. A run that resumed after a long suspension was handed a
    /// brand-new wall-clock budget while keeping its old iteration count.
    ///
    /// Captured once on the first `execute()` entry from
    /// `SystemTime::now()`; on resume the field is already `Some(t)` and
    /// the executor compares against `SystemTime::now()` to detect
    /// budget exhaustion. `#[serde(default)]` so checkpoints written by
    /// pre-iter-6 builds (none expected in production, but defensive)
    /// deserialize as `None` and the executor sets the anchor on first
    /// entry rather than failing to load.
    #[serde(default)]
    pub started_at_unix_ms: Option<u64>,
}

impl LoopExecutionState {
    /// Builds the initial state at the start of a fresh run.
    ///
    /// The `input_cursor` field is populated via
    /// [`LoopInputCursor::origin_for_run`], which binds the cursor to the
    /// active run's `(scope, run_id)`. Callers must therefore hold a valid
    /// [`LoopRunContext`] at the start of every run — there is no
    /// `Default`-shaped constructor because every cursor must name a run.
    pub fn initial_for_run(context: &LoopRunContext) -> Self {
        Self {
            iteration: 0,
            last_checkpoint: None,
            assistant_refs: Vec::new(),
            result_refs: Vec::new(),
            last_gate: None,
            input_cursor: LoopInputCursor::origin_for_run(context),
            surface_version: None,
            recent_call_signatures: BoundedRing::new(),
            recent_failure_kinds: BoundedRing::new(),
            context_state: ContextStrategyState::default(),
            capability_state: CapabilityStrategyState::default(),
            model_state: ModelStrategyState::default(),
            recovery_state: RecoveryStrategyState::default(),
            control_state: ControlStrategyState::default(),
            // Iter-6 finding 1: anchor is set on first `execute()` entry
            // (so tests that build state without entering the executor see
            // `None`). The executor's tick prologue captures
            // `SystemTime::now()` only when this field is `None`,
            // preserving the anchor across resumes.
            started_at_unix_ms: None,
        }
    }

    /// Rehydrates state from a checkpoint payload.
    ///
    /// Payloads must be JSON objects shaped as:
    /// `{ "schema_id": "reborn:default-loop-v1", "state": <LoopExecutionState> }`.
    pub fn from_checkpoint_payload(
        payload: &serde_json::Value,
    ) -> Result<Self, CheckpointPayloadError> {
        let object = payload
            .as_object()
            .ok_or_else(|| CheckpointPayloadError::InvalidField {
                field: "payload",
                reason: "expected checkpoint payload object".to_string(),
            })?;
        let schema_id = object
            .get("schema_id")
            .ok_or(CheckpointPayloadError::MissingField { field: "schema_id" })?;
        let schema_id = schema_id
            .as_str()
            .ok_or_else(|| CheckpointPayloadError::InvalidField {
                field: "schema_id",
                reason: "expected string schema id".to_string(),
            })?;
        if schema_id != CHECKPOINT_SCHEMA_ID {
            return Err(CheckpointPayloadError::SchemaMismatch {
                expected: CHECKPOINT_SCHEMA_ID.to_string(),
                actual: schema_id.to_string(),
            });
        }

        let state = object
            .get("state")
            .ok_or(CheckpointPayloadError::MissingField { field: "state" })?;
        serde_json::from_value(state.clone()).map_err(|error| {
            CheckpointPayloadError::InvalidField {
                field: "state",
                reason: error.to_string(),
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CheckpointMarker {
    pub kind: CheckpointKind,
    pub iteration_at_checkpoint: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointKind {
    BeforeModel,
    BeforeSideEffect,
    BeforeBlock,
    Final,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CheckpointPayloadError {
    #[error("checkpoint payload schema id mismatch: expected `{expected}`, got `{actual}`")]
    SchemaMismatch { expected: String, actual: String },
    #[error("checkpoint payload missing required field `{field}`")]
    MissingField { field: &'static str },
    #[error("checkpoint payload field `{field}` failed validation: {reason}")]
    InvalidField { field: &'static str, reason: String },
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{CapabilityId, TenantId, ThreadId};
    use ironclaw_turns::{
        AgentLoopDriverDescriptor, RunProfileId, RunProfileVersion, TurnId, TurnRunId, TurnScope,
        run_profile::{
            CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy, CheckpointSchemaId,
            ConcurrencyClass, ContextProfileId, LoopDriverId, ModelProfileId,
            RedactedRunProfileProvenance, ResolvedRunProfile, ResourceBudgetPolicy,
            ResourceBudgetTier, RunClassId, RunProfileFingerprint, RuntimeProfileConstraints,
            SchedulingClass, SteeringPolicy,
        },
    };
    use serde_json::json;

    use super::*;

    fn test_run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-loop-state").expect("valid"),
            None,
            None,
            ThreadId::new("thread-loop-state").expect("valid"),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("loop_state_test_driver").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("loop_state_test_checkpoint").expect("valid"),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("loop_state_test_class").expect("valid"),
            profile_id: RunProfileId::default_profile(),
            profile_version: RunProfileVersion::new(1),
            loop_driver: descriptor.clone(),
            checkpoint_schema_id: descriptor
                .checkpoint_schema_id
                .clone()
                .expect("descriptor checkpoint id"),
            checkpoint_schema_version: descriptor
                .checkpoint_schema_version
                .expect("descriptor checkpoint version"),
            model_profile_id: ModelProfileId::new("loop_state_test_model").expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "loop_state_test_capabilities",
            )
            .expect("valid"),
            context_profile_id: ContextProfileId::new("loop_state_test_context").expect("valid"),
            steering_policy: SteeringPolicy {
                allow_steering: false,
                allow_interrupt: true,
                allow_driver_specific_nudges: false,
            },
            cancellation_policy: CancellationPolicy {
                allow_cancel: true,
                require_checkpoint_before_cancel: false,
            },
            checkpoint_policy: CheckpointPolicy {
                require_before_model: false,
                require_before_side_effect: false,
                require_before_block: true,
                max_checkpoint_bytes: 64 * 1024,
                require_final_checkpoint: false,
                allow_no_reply_completion: false,
            },
            resource_budget_policy: ResourceBudgetPolicy {
                tier: ResourceBudgetTier::new("loop_state_test_tier").expect("valid"),
                max_model_calls: 32,
                max_capability_invocations: 64,
            },
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::new("interactive").expect("valid"),
            concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
            resolution_fingerprint: RunProfileFingerprint::new("loop-state-test-fingerprint")
                .expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }

    #[test]
    fn bounded_ring_push_rolls_over_at_capacity() {
        let mut ring = BoundedRing::<u32, 3>::new();
        ring.push(1);
        ring.push(2);
        ring.push(3);
        ring.push(4);

        assert_eq!(ring.iter().copied().collect::<Vec<_>>(), vec![2, 3, 4]);
    }

    #[test]
    fn bounded_ring_most_common_count_respects_window() {
        let mut ring = BoundedRing::<u32, 8>::new();
        for item in [1, 2, 2, 3, 3, 3] {
            ring.push(item);
        }

        assert_eq!(ring.most_common_count_in(0), 0);
        assert_eq!(ring.most_common_count_in(2), 2);
        assert_eq!(ring.most_common_count_in(6), 3);
        assert_eq!(ring.most_common_count_in(20), 3);
    }

    #[test]
    fn bounded_ring_same_run_length_counts_trailing_run() {
        let empty = BoundedRing::<u32, 4>::new();
        assert_eq!(empty.same_run_length(), 0);

        let mut distinct = BoundedRing::<u32, 4>::new();
        distinct.push(1);
        distinct.push(2);
        distinct.push(3);
        assert_eq!(distinct.same_run_length(), 1);

        let mut run = BoundedRing::<u32, 8>::new();
        for item in [1, 2, 3, 3, 3] {
            run.push(item);
        }
        assert_eq!(run.same_run_length(), 3);
    }

    #[test]
    fn capability_call_signature_is_stable_under_key_reordering() {
        let capability = CapabilityId::new("demo.echo").unwrap();
        let reordered = CapabilityId::new("demo.echo").unwrap();
        let first = CapabilityCallSignature::from_call(
            capability,
            &json!({"b": 2, "a": {"d": false, "c": [1, null]}}),
        );
        let second = CapabilityCallSignature::from_call(
            reordered,
            &json!({"a": {"c": [1, null], "d": false}, "b": 2}),
        );

        assert_eq!(first, second);
    }

    #[test]
    fn initial_state_is_value_equal_across_calls() {
        let context = test_run_context();
        assert_eq!(
            LoopExecutionState::initial_for_run(&context),
            LoopExecutionState::initial_for_run(&context)
        );
    }

    #[test]
    fn loop_execution_state_round_trips_through_json() {
        let context = test_run_context();
        let state = LoopExecutionState::initial_for_run(&context);
        let value = serde_json::to_value(&state).unwrap();
        let restored: LoopExecutionState = serde_json::from_value(value).unwrap();

        assert_eq!(restored, state);
    }

    #[test]
    fn checkpoint_payload_rejects_schema_mismatch() {
        let context = test_run_context();
        let payload = json!({
            "schema_id": "reborn:other-loop-v1",
            "state": LoopExecutionState::initial_for_run(&context)
        });

        assert_eq!(
            LoopExecutionState::from_checkpoint_payload(&payload),
            Err(CheckpointPayloadError::SchemaMismatch {
                expected: CHECKPOINT_SCHEMA_ID.to_string(),
                actual: "reborn:other-loop-v1".to_string(),
            })
        );
    }

    #[test]
    fn checkpoint_payload_rejects_bounded_ring_over_capacity() {
        let context = test_run_context();
        let mut state =
            serde_json::to_value(LoopExecutionState::initial_for_run(&context)).unwrap();
        let recent_call_signatures = state
            .get_mut("recent_call_signatures")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|object| object.get_mut("items"))
            .and_then(serde_json::Value::as_array_mut)
            .unwrap();
        for index in 0..9 {
            recent_call_signatures.push(json!(CapabilityCallSignature::from_call(
                CapabilityId::new(format!("demo.echo_{index}")).unwrap(),
                &json!({ "index": index })
            )));
        }
        let payload = json!({
            "schema_id": CHECKPOINT_SCHEMA_ID,
            "state": state,
        });

        let result = LoopExecutionState::from_checkpoint_payload(&payload);

        assert!(matches!(
            result,
            Err(CheckpointPayloadError::InvalidField { field: "state", .. })
        ));
    }
}
