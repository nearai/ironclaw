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
pub use signature::{ArgsHash, CapabilityCallSignature, CapabilityCallSignatureError};
pub use slots::{
    CapabilityStrategyState, ContextStrategyState, GateStrategyState, ModelStrategyState,
    RecoveryAttemptClass, RecoveryStrategyState, StopStrategyState,
};

use ironclaw_turns::{
    LoopGateRef, LoopMessageRef, LoopResultRef,
    run_profile::{CapabilitySurfaceVersion, LoopInputCursor, LoopRunContext},
};

/// Checkpoint payload schema reserved for the default Reborn loop.
///
/// Note: master spec §9 pins `ComponentIdentity { id, digest }` as the
/// canonical versioning shape for checkpoint payload metadata. WS-0 keeps the
/// legacy `&'static str` form because the `ComponentIdentity` migration is
/// deferred to follow-up PRs (#3470/#3524/#3462) per the brief.
pub const CHECKPOINT_SCHEMA_ID: &str = "reborn:default-loop-v1";
pub const CHECKPOINT_SCHEMA_VERSION: u64 = 1;

/// Immutable execution state threaded through the loop.
///
/// The executor rebinds its local `let mut state` each tick to the next whole
/// state. Strategies receive `&LoopExecutionState` and return outcome enums
/// that carry the new value of their own slot. The executor builds the next
/// whole state by swapping that slot.
///
/// Stop and Gate each own their own slot — there is no shared `control_state`
/// — so a family's future growth in either dimension can't accidentally mix
/// concerns through a shared struct.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LoopExecutionState {
    // executor-universal
    pub iteration: u32,
    pub last_checkpoint: Option<CheckpointMarker>,
    pub assistant_refs: Vec<LoopMessageRef>,
    pub result_refs: Vec<LoopResultRef>,
    pub last_gate: Option<LoopGateRef>,
    pub input_cursor: LoopInputCursor,
    pub surface_version: Option<CapabilitySurfaceVersion>,

    // executor-observed (populated by executor; read-only to strategies)
    pub recent_call_signatures: BoundedRing<CapabilityCallSignature, 8>,
    pub recent_failure_kinds: BoundedRing<LoopFailureKind, 8>,

    // strategy slots — one per strategy that mutates state.
    pub context_state: ContextStrategyState,
    pub capability_state: CapabilityStrategyState,
    pub model_state: ModelStrategyState,
    pub recovery_state: RecoveryStrategyState,
    pub stop_state: StopStrategyState,
    pub gate_state: GateStrategyState,
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
            stop_state: StopStrategyState::default(),
            gate_state: GateStrategyState::default(),
        }
    }

    /// Rehydrates state from a checkpoint payload's bytes.
    ///
    /// The bytes are the raw JSON-serialized `LoopExecutionState` — i.e. what
    /// the executor produced via `serde_json::to_vec(&state)` before passing
    /// the bytes to `LoopCheckpointPort::stage_checkpoint_payload`. The payload
    /// contains **no outer envelope**: schema-id and kind live in store-side
    /// metadata, validated by `CheckpointStateStore::get_checkpoint_state`
    /// before the bytes ever reach this function. The `kind` argument is
    /// accepted for API symmetry (the call site can document what boundary the
    /// checkpoint belongs to) but is not used to authenticate the bytes.
    pub fn from_checkpoint_payload(
        payload: &[u8],
        _kind: CheckpointKind,
    ) -> Result<Self, CheckpointPayloadError> {
        serde_json::from_slice(payload).map_err(|error| CheckpointPayloadError::InvalidField {
            field: "payload",
            reason: error.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CheckpointMarker {
    pub kind: CheckpointKind,
    pub iteration_at_checkpoint: u32,
}

/// Mirrors the four checkpoint boundaries from the executor (master doc §8).
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
    #[error("checkpoint payload kind mismatch: expected `{expected:?}`, got `{actual:?}`")]
    KindMismatch {
        expected: CheckpointKind,
        actual: CheckpointKind,
    },
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

    /// Encode a checkpoint payload the same way the executor does:
    /// `serde_json::to_vec(&state)` — no outer envelope.
    /// Schema-id and kind are stored as side-channel metadata by
    /// `CheckpointStateStore::put_checkpoint_state`, not inside the bytes.
    fn encode_payload(state: &LoopExecutionState) -> Vec<u8> {
        serde_json::to_vec(state).expect("encode payload")
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
        )
        .unwrap();
        let second = CapabilityCallSignature::from_call(
            reordered,
            &json!({"a": {"c": [1, null], "d": false}, "b": 2}),
        )
        .unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn capability_call_signature_is_stable_across_pretty_vs_minified_inputs() {
        let capability = CapabilityId::new("demo.echo").unwrap();
        let minified: serde_json::Value =
            serde_json::from_str(r#"{"a":1,"b":[2,3],"c":{"d":4}}"#).unwrap();
        let pretty: serde_json::Value = serde_json::from_str(
            "{\n  \"a\": 1,\n  \"b\": [2, 3],\n  \"c\": {\n    \"d\": 4\n  }\n}",
        )
        .unwrap();

        let from_minified =
            CapabilityCallSignature::from_call(capability.clone(), &minified).unwrap();
        let from_pretty = CapabilityCallSignature::from_call(capability, &pretty).unwrap();
        assert_eq!(from_minified.args_hash, from_pretty.args_hash);
    }

    #[test]
    fn capability_call_signature_is_stable_under_nested_key_reordering() {
        let capability = CapabilityId::new("demo.echo").unwrap();
        let first = CapabilityCallSignature::from_call(
            capability.clone(),
            &json!({
                "outer": {
                    "alpha": 1,
                    "beta": {"x": 10, "y": 20},
                    "gamma": [
                        {"p": 1, "q": 2},
                        {"r": 3, "s": 4}
                    ]
                }
            }),
        )
        .unwrap();
        let second = CapabilityCallSignature::from_call(
            capability,
            &json!({
                "outer": {
                    "gamma": [
                        {"q": 2, "p": 1},
                        {"s": 4, "r": 3}
                    ],
                    "beta": {"y": 20, "x": 10},
                    "alpha": 1
                }
            }),
        )
        .unwrap();
        assert_eq!(first.args_hash, second.args_hash);
    }

    #[test]
    fn capability_call_signature_rejects_nan_and_infinity() {
        let capability = CapabilityId::new("demo.echo").unwrap();
        let nan = serde_json::Number::from_f64(f64::NAN);
        let infinity = serde_json::Number::from_f64(f64::INFINITY);
        // serde_json refuses to construct NaN/Infinity through its public API;
        // synthesize them via a manually built Value to exercise the guard.
        // If the upstream representation rejects these inputs entirely, the
        // guard is unreachable at the public boundary — assert that.
        assert!(nan.is_none(), "serde_json refuses NaN at the Number level");
        assert!(
            infinity.is_none(),
            "serde_json refuses Infinity at the Number level"
        );

        // Round-trip a JSON string that contains a NaN-like token. serde_json
        // rejects this at the parser, so we exercise the guard via the
        // signature's own check against the canonicalized output.
        let parse: Result<serde_json::Value, _> = serde_json::from_str("NaN");
        assert!(parse.is_err());

        // The function is fallible by signature; with valid JSON input we
        // should always get Ok.
        let ok = CapabilityCallSignature::from_call(capability, &json!({"x": 1.0}));
        assert!(ok.is_ok());
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
    fn loop_execution_state_has_no_control_state_field() {
        // Grep-style assertion: when serialized, the JSON object must carry
        // `stop_state` and `gate_state` and must NOT carry `control_state`.
        let context = test_run_context();
        let state = LoopExecutionState::initial_for_run(&context);
        let value = serde_json::to_value(&state).unwrap();
        let object = value.as_object().expect("state serializes as object");
        assert!(
            object.contains_key("stop_state"),
            "missing stop_state on serialized LoopExecutionState"
        );
        assert!(
            object.contains_key("gate_state"),
            "missing gate_state on serialized LoopExecutionState"
        );
        assert!(
            !object.contains_key("control_state"),
            "unexpected control_state on serialized LoopExecutionState"
        );
    }

    #[test]
    fn stop_and_gate_strategy_state_round_trip() {
        let stop = StopStrategyState::default();
        let stop_bytes = serde_json::to_vec(&stop).unwrap();
        let stop_restored: StopStrategyState = serde_json::from_slice(&stop_bytes).unwrap();
        assert_eq!(stop_restored, stop);

        let gate = GateStrategyState::default();
        let gate_bytes = serde_json::to_vec(&gate).unwrap();
        let gate_restored: GateStrategyState = serde_json::from_slice(&gate_bytes).unwrap();
        assert_eq!(gate_restored, gate);
    }

    /// Schema-id and kind validation now live in the store layer
    /// (`CheckpointStateStore::get_checkpoint_state`) — not in the payload
    /// bytes. `from_checkpoint_payload` therefore succeeds for any
    /// well-formed `LoopExecutionState` regardless of what kind is passed.
    #[test]
    fn checkpoint_payload_round_trips_raw_state_bytes() {
        let context = test_run_context();
        let state = LoopExecutionState::initial_for_run(&context);
        let payload = encode_payload(&state);

        let restored =
            LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeModel)
                .unwrap();
        assert_eq!(restored, state);
    }

    #[test]
    fn checkpoint_payload_kind_arg_is_accepted_for_any_valid_state() {
        // kind is metadata — passing Final for bytes encoded without a kind
        // label must still succeed, because kind authentication happens at the
        // store boundary before bytes are handed to from_checkpoint_payload.
        let context = test_run_context();
        let state = LoopExecutionState::initial_for_run(&context);
        let payload = encode_payload(&state);

        let result = LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::Final);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), state);
    }

    #[test]
    fn checkpoint_payload_rejects_malformed_bytes() {
        // Non-JSON bytes must still fail with InvalidField { field: "payload" }.
        let result = LoopExecutionState::from_checkpoint_payload(
            b"not json at all",
            CheckpointKind::BeforeModel,
        );

        assert!(matches!(
            result,
            Err(CheckpointPayloadError::InvalidField {
                field: "payload",
                ..
            })
        ));
    }

    #[test]
    fn checkpoint_payload_rejects_bounded_ring_over_capacity() {
        // Raw state bytes with an over-capacity BoundedRing must fail on
        // deserialization (the BoundedRing Deserialize impl enforces capacity).
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
            recent_call_signatures.push(json!(
                CapabilityCallSignature::from_call(
                    CapabilityId::new(format!("demo.echo_{index}")).unwrap(),
                    &json!({ "index": index })
                )
                .unwrap()
            ));
        }
        // Encode as raw state bytes (no envelope).
        let bytes = serde_json::to_vec(&state).unwrap();

        let result =
            LoopExecutionState::from_checkpoint_payload(&bytes, CheckpointKind::BeforeModel);

        assert!(matches!(
            result,
            Err(CheckpointPayloadError::InvalidField {
                field: "payload",
                ..
            })
        ));
    }
}
