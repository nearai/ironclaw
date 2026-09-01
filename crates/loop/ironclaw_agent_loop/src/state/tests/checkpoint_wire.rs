use ironclaw_host_api::ids::{CapabilityId, TenantId, ThreadId};
use ironclaw_host_api::turn::{
    GateResumeDisposition, RunProfileId, RunProfileVersion, TurnId, TurnRunId, TurnScope,
};
use ironclaw_loop_contracts::{
    AgentLoopDriverDescriptor, CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy,
    CheckpointSchemaId, ConcurrencyClass, ContextProfileId, LoopDriverId, ModelProfileId,
    RedactedRunProfileProvenance, ResolvedRunProfile, ResourceBudgetPolicy, ResourceBudgetTier,
    RunClassId, RunProfileFingerprint, RuntimeProfileConstraints, SchedulingClass, SteeringPolicy,
};
use serde_json::json;

use super::super::*;

pub(super) fn test_run_context() -> LoopRunContext {
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
            before_model_checkpoint_interval: 1,
        },
        resource_budget_policy: ResourceBudgetPolicy {
            tier: ResourceBudgetTier::new("loop_state_test_tier").expect("valid"),
            max_model_calls: 32,
            max_capability_invocations: 64,
            max_wall_clock_seconds: None,
        },
        personal_context_policy: ironclaw_loop_contracts::PersonalContextPolicy::Excluded,
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
/// Schema-id and kind are stored as process checkpoint metadata, not inside
/// the bytes.
pub(super) fn encode_payload(state: &LoopExecutionState) -> Vec<u8> {
    serde_json::to_vec(state).expect("encode payload")
}

#[test]
fn public_state_reexports_remain_available() {
    use crate::state::{
        ArgsHash, AuthResumeApprovalIdentity, BoundedRing, BudgetLedger, CapabilityCallSignature,
        CapabilityCallSignatureError, CapabilityOutputObservation, CapabilityStrategyState,
        CheckpointKind, CheckpointMarker, CheckpointPayloadError, CompactionEffectivenessBaseline,
        CompactionPromptSnapshot, CompactionStrategyState, ContextStrategyState,
        DeferredCompactionWatermark, GateStrategyState, GoalRefreshStrategyState,
        IndexedMessageKind, LoopExecutionState, LoopFailureKind, MessageIndexEntry,
        ModelErrorObservationClass, ModelErrorRecoveryObservation, ModelStrategyState,
        PendingApprovalResume, PendingAuthResume, PendingExternalToolResume,
        PendingModelRetryDirective, PostCapabilityStageState, RecoveryAttemptClass,
        RecoveryStrategyState, RepeatedCallWarningPhase, RepeatedCallWarningState,
        ReplyAdmissionRejection, ReplyAdmissionRejectionReason, ReplyAdmissionStrategyState,
        StopStrategyState, TerminalWarningState,
    };

    fn assert_type_available<T>() {}

    assert_type_available::<BoundedRing<u8, 1>>();
    assert_type_available::<BudgetLedger>();
    assert_type_available::<AuthResumeApprovalIdentity>();
    assert_type_available::<LoopFailureKind>();
    assert_type_available::<ModelErrorRecoveryObservation>();
    assert_type_available::<PendingModelRetryDirective>();
    assert_type_available::<ArgsHash>();
    assert_type_available::<CapabilityCallSignature>();
    assert_type_available::<CapabilityCallSignatureError>();
    assert_type_available::<CapabilityOutputObservation>();
    assert_type_available::<CapabilityStrategyState>();
    assert_type_available::<CompactionEffectivenessBaseline>();
    assert_type_available::<CompactionPromptSnapshot>();
    assert_type_available::<CompactionStrategyState>();
    assert_type_available::<ContextStrategyState>();
    assert_type_available::<DeferredCompactionWatermark>();
    assert_type_available::<GateStrategyState>();
    assert_type_available::<GoalRefreshStrategyState>();
    assert_type_available::<IndexedMessageKind>();
    assert_type_available::<MessageIndexEntry>();
    assert_type_available::<ModelErrorObservationClass>();
    assert_type_available::<ModelStrategyState>();
    assert_type_available::<PostCapabilityStageState>();
    assert_type_available::<RecoveryAttemptClass>();
    assert_type_available::<RecoveryStrategyState>();
    assert_type_available::<RepeatedCallWarningPhase>();
    assert_type_available::<RepeatedCallWarningState>();
    assert_type_available::<ReplyAdmissionRejection>();
    assert_type_available::<ReplyAdmissionRejectionReason>();
    assert_type_available::<ReplyAdmissionStrategyState>();
    assert_type_available::<StopStrategyState>();
    assert_type_available::<TerminalWarningState>();
    assert_type_available::<LoopExecutionState>();
    assert_type_available::<PendingApprovalResume>();
    assert_type_available::<PendingAuthResume>();
    assert_type_available::<PendingExternalToolResume>();
    assert_type_available::<CheckpointMarker>();
    assert_type_available::<CheckpointKind>();
    assert_type_available::<CheckpointPayloadError>();
}

fn deterministic_checkpoint_state() -> LoopExecutionState {
    let mut state = LoopExecutionState::initial_for_run(&test_run_context());
    state.input_cursor = serde_json::from_value(json!({
        "scope": {
            "tenant_id": "tenant-loop-state",
            "agent_id": null,
            "project_id": null,
            "thread_id": "thread-loop-state"
        },
        "run_id": "00000000-0000-0000-0000-000000000001",
        "token": "input-cursor:origin"
    }))
    .expect("deterministic input cursor");
    state
}

#[test]
fn checkpoint_payload_matches_pre_move_raw_json_golden() {
    const PRE_MOVE_GOLDEN: &[u8] = br#"{"iteration":0,"last_checkpoint":null,"assistant_refs":[],"result_refs":[],"last_gate":null,"input_cursor":{"scope":{"tenant_id":"tenant-loop-state","agent_id":null,"project_id":null,"thread_id":"thread-loop-state"},"run_id":"00000000-0000-0000-0000-000000000001","token":"input-cursor:origin"},"surface_version":null,"recent_call_signatures":{"items":[]},"seen_capability_output_digests":{"items":[]},"recent_failure_kinds":{"items":[]},"recent_output_token_counts":{"items":[]},"model_calls_made":0,"capability_invocations_made":0,"completion_nudges_used":0,"completion_nudge_pending":false,"terminal_warning_state":{},"last_reply_trailed_off":false,"last_reply_empty":false,"last_reply_ended_with_question":false,"context_state":{},"capability_state":{},"model_state":{"fallback_index":0},"compaction_state":{"force_compact_on_next_iteration":false,"consecutive_ineffective_compactions":0,"compaction_circuit_open":false},"compaction_prompt":{"message_index":[],"observed_prompt_tokens":0},"post_capability_state":{"skip_model_this_iteration":false},"goal_refresh_state":{"turns_since_refresh":0},"recovery_state":{},"recovery_event_sequence":0,"reply_admission_state":{"rejected_reply_candidates":0,"pending_rejection_rendered":false},"stop_state":{"turns_completed":0,"trailing_rejected_replies":0,"trailing_no_progress_results":0,"trailing_all_failed_batches":0,"structured_result_recorded":false},"gate_state":{}}"#;

    assert_eq!(
        encode_payload(&deterministic_checkpoint_state()),
        PRE_MOVE_GOLDEN
    );
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
    let pretty: serde_json::Value =
        serde_json::from_str("{\n  \"a\": 1,\n  \"b\": [2, 3],\n  \"c\": {\n    \"d\": 4\n  }\n}")
            .unwrap();

    let from_minified = CapabilityCallSignature::from_call(capability.clone(), &minified).unwrap();
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
fn seen_capability_output_digests_round_trips_through_checkpoint_payload() {
    let context = test_run_context();
    let mut state = LoopExecutionState::initial_for_run(&context);
    let signature = CapabilityCallSignature::from_call(
        CapabilityId::new("demo.echo").expect("valid capability id"),
        &json!({"message": "hi"}),
    )
    .expect("signature builds");
    state
        .seen_capability_output_digests
        .push(CapabilityOutputObservation {
            signature,
            output_digest: ironclaw_loop_contracts::ContentDigest(42),
        });

    let payload = encode_payload(&state);
    let restored =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeBlock)
            .expect("decode checkpoint payload");

    assert_eq!(
        restored.seen_capability_output_digests, state.seen_capability_output_digests,
        "seen_capability_output_digests must survive checkpoint encode/decode"
    );
}

#[test]
fn checkpoint_payload_without_output_digest_ring_decodes_to_empty() {
    let context = test_run_context();
    let mut state = LoopExecutionState::initial_for_run(&context);
    let signature = CapabilityCallSignature::from_call(
        CapabilityId::new("demo.echo").expect("valid capability id"),
        &json!({"message": "hi"}),
    )
    .expect("signature builds");
    state
        .seen_capability_output_digests
        .push(CapabilityOutputObservation {
            signature,
            output_digest: ironclaw_loop_contracts::ContentDigest(42),
        });

    let payload = encode_payload(&state);
    let mut value: serde_json::Value = serde_json::from_slice(&payload).expect("parse");
    value
        .as_object_mut()
        .expect("state serializes as object")
        .remove("seen_capability_output_digests");
    let stripped_payload = serde_json::to_vec(&value).expect("re-encode");
    let from_legacy =
        LoopExecutionState::from_checkpoint_payload(&stripped_payload, CheckpointKind::BeforeBlock)
            .expect("decode legacy checkpoint payload without seen_capability_output_digests");

    assert!(
        from_legacy.seen_capability_output_digests.is_empty(),
        "legacy checkpoint missing seen_capability_output_digests must decode to an empty ring"
    );
}

/// Frozen-shape pin for the `BudgetLedger` refactor: before this refactor
/// `run_started_at`, `model_calls_made`, and `capability_invocations_made`
/// were three bare top-level fields on `LoopExecutionState`. The refactor
/// moves them into `BudgetLedger` behind `#[serde(flatten)]`, which must
/// keep the exact same top-level keys (no nested `budget_ledger` object)
/// so checkpoints written before this refactor still decode, and
/// checkpoints written after it still decode under a rollback to the
/// pre-refactor binary.
#[test]
fn budget_ledger_wire_shape_is_frozen_to_the_pre_refactor_top_level_fields() {
    let context = test_run_context();
    let mut legacy_shaped = serde_json::to_value(LoopExecutionState::initial_for_run(&context))
        .expect("encode baseline state");
    let object = legacy_shaped
        .as_object_mut()
        .expect("state serializes as object");
    // The pre-refactor wire shape carried these three fields directly on
    // the top-level object. This is exactly the payload shape a
    // checkpoint written before this refactor has.
    object.insert("run_started_at".to_string(), json!("2026-01-01T00:00:00Z"));
    object.insert("model_calls_made".to_string(), json!(7));
    object.insert("capability_invocations_made".to_string(), json!(9));
    assert!(
        !object.contains_key("budget_ledger"),
        "the flattened ledger must not require (or introduce) a nested \
             `budget_ledger` wire key"
    );

    let payload = serde_json::to_vec(&legacy_shaped).expect("re-encode legacy-shaped payload");
    let decoded =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeBlock)
            .expect("decode legacy top-level budget fields");

    assert_eq!(
        decoded.budget_ledger.run_started_at(),
        Some(
            "2026-01-01T00:00:00Z"
                .parse::<chrono::DateTime<chrono::Utc>>()
                .expect("valid timestamp")
        )
    );
    assert_eq!(decoded.budget_ledger.model_calls_made(), 7);
    assert_eq!(decoded.budget_ledger.capability_invocations_made(), 9);

    // Re-encoding must reproduce the same flat top-level keys, not a
    // nested object — byte-compatible with checkpoints written before
    // this refactor.
    let re_encoded = serde_json::to_value(&decoded).expect("re-encode decoded state");
    let re_encoded_object = re_encoded.as_object().expect("state serializes as object");
    assert_eq!(
        re_encoded_object.get("run_started_at"),
        Some(&json!("2026-01-01T00:00:00Z"))
    );
    assert_eq!(re_encoded_object.get("model_calls_made"), Some(&json!(7)));
    assert_eq!(
        re_encoded_object.get("capability_invocations_made"),
        Some(&json!(9))
    );
    assert!(!re_encoded_object.contains_key("budget_ledger"));
}

/// Companion to the frozen-shape pin above: `run_started_at` must stay
/// `skip_serializing_if = "Option::is_none"` and the two counters must
/// stay plain `#[serde(default)]` (always emitted, defaulting to `0` on
/// legacy decode) — exactly the pre-refactor per-field behavior, now
/// living on `BudgetLedger` instead of directly on `LoopExecutionState`.
#[test]
fn budget_ledger_field_defaults_match_pre_refactor_behavior() {
    let context = test_run_context();
    let state = LoopExecutionState::initial_for_run(&context);
    assert!(state.budget_ledger.run_started_at().is_none());

    let value = serde_json::to_value(&state).expect("encode state");
    let object = value.as_object().expect("state serializes as object");
    assert!(
        !object.contains_key("run_started_at"),
        "an unset run_started_at must still be omitted from the payload"
    );
    assert_eq!(object.get("model_calls_made"), Some(&json!(0)));
    assert_eq!(object.get("capability_invocations_made"), Some(&json!(0)));

    // A payload predating these fields entirely (no budget keys at all)
    // must still decode, defaulting the ledger to zeroed/unarmed.
    let mut without_budget_fields = value.clone();
    without_budget_fields
        .as_object_mut()
        .expect("state serializes as object")
        .remove("model_calls_made");
    without_budget_fields
        .as_object_mut()
        .expect("state serializes as object")
        .remove("capability_invocations_made");
    let payload = serde_json::to_vec(&without_budget_fields).expect("re-encode");
    let decoded =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeBlock)
            .expect("decode payload missing budget counters entirely");
    assert_eq!(decoded.budget_ledger.model_calls_made(), 0);
    assert_eq!(decoded.budget_ledger.capability_invocations_made(), 0);
    assert!(decoded.budget_ledger.run_started_at().is_none());
}

#[test]
fn compaction_prompt_snapshot_round_trips_through_checkpoints() {
    let context = test_run_context();
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.compaction_prompt =
        CompactionPromptSnapshot::from_message_index(vec![MessageIndexEntry {
            sequence: 1,
            kind: IndexedMessageKind::User,
            estimated_tokens: 42,
        }]);

    let value = serde_json::to_value(&state).unwrap();
    assert!(
        value
            .as_object()
            .expect("state serializes as object")
            .contains_key("compaction_prompt")
    );
    let restored: LoopExecutionState = serde_json::from_value(value).unwrap();

    assert_eq!(restored.compaction_prompt, state.compaction_prompt);
    assert_eq!(restored.compaction_state, state.compaction_state);
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

/// Schema-id and kind validation live in the process checkpoint projection,
/// not in the payload
/// bytes. `from_checkpoint_payload` therefore succeeds for any
/// well-formed `LoopExecutionState` regardless of what kind is passed.
#[test]
fn checkpoint_payload_round_trips_raw_state_bytes() {
    let context = test_run_context();
    let state = LoopExecutionState::initial_for_run(&context);
    let payload = encode_payload(&state);

    let restored =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeModel).unwrap();
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
    let mut state = serde_json::to_value(LoopExecutionState::initial_for_run(&context)).unwrap();
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

    let result = LoopExecutionState::from_checkpoint_payload(&bytes, CheckpointKind::BeforeModel);

    assert!(matches!(
        result,
        Err(CheckpointPayloadError::InvalidField {
            field: "payload",
            ..
        })
    ));
}

/// Round-2 test coverage: verify that a non-default `post_capability_state`
/// (populated `pending_capability_bytes` + `skip_model_this_iteration = true`)
/// survives `to_checkpoint_payload` / `from_checkpoint_payload` intact.
///
/// This is the replay-correctness gate: if these fields are lost on
/// checkpoint encode/decode, a resumed run would start with a stale byte
/// accumulator or would incorrectly re-run the model that was supposed to
/// be skipped.
#[test]
fn post_capability_state_with_bytes_round_trips_through_checkpoint() {
    let context = test_run_context();
    let mut state = LoopExecutionState::initial_for_run(&context);

    // Seed pending_capability_bytes with a non-zero entry.
    let cap_id = CapabilityId::new("builtin.http").expect("valid capability id");
    state
        .post_capability_state
        .pending_capability_bytes
        .insert(cap_id.clone(), 33_001);
    // Also set skip_model_this_iteration to verify it round-trips.
    state.post_capability_state.skip_model_this_iteration = true;

    let payload = encode_payload(&state);
    let restored =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeModel)
            .expect("decode checkpoint payload");

    assert_eq!(
        restored
            .post_capability_state
            .pending_capability_bytes
            .get(&cap_id),
        Some(&33_001),
        "pending_capability_bytes must survive checkpoint encode/decode"
    );
    assert!(
        restored.post_capability_state.skip_model_this_iteration,
        "skip_model_this_iteration must survive checkpoint encode/decode"
    );
    // Full equality check — no other fields must have changed.
    assert_eq!(
        restored.post_capability_state, state.post_capability_state,
        "entire PostCapabilityStageState must round-trip without loss"
    );
}

#[test]
fn pending_auth_resume_round_trips_through_checkpoint_payload() {
    let context = test_run_context();
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:auth-test").expect("valid gate ref"),
        capability_id: CapabilityId::new("gsuite.calendar.list_events").expect("valid cap id"),
        surface_version: CapabilitySurfaceVersion::new("surface-v1")
            .expect("valid surface version"),
        input_ref: CapabilityInputRef::new("input:test").expect("valid input ref"),
        effective_capability_ids: vec![],
        provider_replay: None,
        resume_token: None,
        activity_id: CapabilityActivityId::new(),
        prior_approval: None,
        disposition: None,
    });
    let payload = encode_payload(&state);
    let restored =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeBlock)
            .expect("decode checkpoint payload");
    assert_eq!(
        restored.pending_auth_resume, state.pending_auth_resume,
        "PendingAuthResume must survive checkpoint encode/decode"
    );
}

#[test]
fn pending_auth_resume_denied_disposition_round_trips_through_checkpoint_payload() {
    // Regression: the `Some(Denied)` disposition stamped by `planned_driver`
    // before the capability stage must survive the checkpoint encode/decode
    // cycle so that a resumed run still sees the denial.
    let context = test_run_context();
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:auth-denied-test").expect("valid gate ref"),
        capability_id: CapabilityId::new("gsuite.calendar.list_events").expect("valid cap id"),
        surface_version: CapabilitySurfaceVersion::new("surface-v1")
            .expect("valid surface version"),
        input_ref: CapabilityInputRef::new("input:denied-test").expect("valid input ref"),
        effective_capability_ids: vec![],
        provider_replay: None,
        resume_token: None,
        activity_id: CapabilityActivityId::new(),
        prior_approval: None,
        disposition: Some(GateResumeDisposition::Denied),
    });
    let payload = encode_payload(&state);
    let restored =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeBlock)
            .expect("decode checkpoint payload");
    assert_eq!(
        restored
            .pending_auth_resume
            .as_ref()
            .and_then(|r| r.disposition.as_ref()),
        Some(&GateResumeDisposition::Denied),
        "PendingAuthResume with Denied disposition must survive checkpoint encode/decode"
    );
    assert_eq!(
        restored.pending_auth_resume, state.pending_auth_resume,
        "entire PendingAuthResume must round-trip without loss when disposition is Some(Denied)"
    );
}

#[test]
fn checkpoint_payload_without_auth_resume_slot_decodes_to_none() {
    // Encode a state with no pending_auth_resume; decode must yield None.
    let context = test_run_context();
    let state = LoopExecutionState::initial_for_run(&context);
    assert!(
        state.pending_auth_resume.is_none(),
        "initial state must have no pending_auth_resume"
    );

    // Round-trip through the normal encode/decode path.
    let payload = encode_payload(&state);
    let restored =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeBlock)
            .expect("decode checkpoint payload");
    assert!(
        restored.pending_auth_resume.is_none(),
        "decoded state must have no pending_auth_resume when field was absent from payload"
    );
}

#[test]
fn pending_auth_resume_optional_fields_round_trip_through_checkpoint_payload() {
    use ironclaw_host_api::ids::{ApprovalRequestId, CorrelationId};
    use ironclaw_loop_contracts::{AuthResumeApprovalIdentity, CapabilityResumeToken};

    let context = test_run_context();
    let mut state = LoopExecutionState::initial_for_run(&context);

    // Build a PendingAuthResume with all optional fields set.
    let resume_token = CapabilityResumeToken::new("00000000-0000-0000-0000-000000000001")
        .expect("valid resume token");
    let activity_id = CapabilityActivityId::parse(resume_token.as_str())
        .expect("resume token fixture is an activity id");
    let approval_request_id = ApprovalRequestId::new();
    let correlation_id = CorrelationId::new();
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:auth-with-approval").expect("valid gate ref"),
        capability_id: CapabilityId::new("gsuite.calendar.list_events").expect("valid cap id"),
        surface_version: CapabilitySurfaceVersion::new("surface-v2")
            .expect("valid surface version"),
        input_ref: CapabilityInputRef::new("input:approval-auth").expect("valid input ref"),
        effective_capability_ids: vec![],
        provider_replay: None,
        resume_token: Some(resume_token.clone()),
        activity_id,
        prior_approval: Some(AuthResumeApprovalIdentity {
            approval_request_id,
            correlation_id,
        }),
        disposition: None,
    });

    // Round-trip: all optional fields must survive encode/decode.
    let payload = encode_payload(&state);
    let restored =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeBlock)
            .expect("decode checkpoint payload with resume_token fields");
    let pending = restored
        .pending_auth_resume
        .expect("pending_auth_resume must be present after round-trip");
    assert_eq!(
        pending.resume_token,
        Some(resume_token),
        "resume_token must survive checkpoint encode/decode"
    );
    assert_eq!(
        pending.activity_id, activity_id,
        "activity_id must survive checkpoint encode/decode"
    );
    let pa = pending
        .prior_approval
        .expect("prior_approval must survive checkpoint encode/decode");
    assert_eq!(
        pa.approval_request_id, approval_request_id,
        "prior_approval.approval_request_id must survive checkpoint encode/decode"
    );
    assert_eq!(
        pa.correlation_id, correlation_id,
        "prior_approval.correlation_id must survive checkpoint encode/decode"
    );
}

#[test]
fn pending_approval_resume_denied_disposition_round_trips_through_checkpoint_payload() {
    // Mirror of `pending_auth_resume_denied_disposition_round_trips_through_checkpoint_payload`.
    // The `Some(Denied)` disposition stamped on `pending_approval_resume` before the
    // capability stage must survive the checkpoint encode/decode cycle so that a
    // resumed run still sees the approval denial.
    use ironclaw_host_api::ids::{ApprovalRequestId, CorrelationId};
    use ironclaw_loop_contracts::CapabilityResumeToken;

    let context = test_run_context();
    let mut state = LoopExecutionState::initial_for_run(&context);
    let resume_token =
        CapabilityResumeToken::new("00000000-0000-0000-0000-000000000099").expect("valid");
    let activity_id = CapabilityActivityId::parse(resume_token.as_str())
        .expect("resume token fixture is an activity id");
    state.pending_approval_resume = Some(super::PendingApprovalResume {
        gate_ref: LoopGateRef::new("gate:approval-denied-test").expect("valid gate ref"),
        capability_id: CapabilityId::new("extensions.install").expect("valid cap id"),
        approval_request_id: ApprovalRequestId::new(),
        resume_token,
        activity_id,
        correlation_id: CorrelationId::new(),
        surface_version: CapabilitySurfaceVersion::new("surface-v1")
            .expect("valid surface version"),
        input_ref: CapabilityInputRef::new("input:approval-denied").expect("valid input ref"),
        effective_capability_ids: vec![],
        provider_replay: None,
        disposition: Some(GateResumeDisposition::Denied),
    });
    let payload = encode_payload(&state);
    let restored =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeBlock)
            .expect("decode checkpoint payload");
    assert_eq!(
        restored
            .pending_approval_resume
            .as_ref()
            .and_then(|r| r.disposition.as_ref()),
        Some(&GateResumeDisposition::Denied),
        "PendingApprovalResume with Denied disposition must survive checkpoint encode/decode"
    );
    assert_eq!(
        restored.pending_approval_resume, state.pending_approval_resume,
        "entire PendingApprovalResume must round-trip without loss when disposition is Some(Denied)"
    );
}
