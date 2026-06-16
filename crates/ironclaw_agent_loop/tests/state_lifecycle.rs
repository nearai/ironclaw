use ironclaw_agent_loop::{
    executor::{AgentLoopExecutor, CanonicalAgentLoopExecutor},
    families,
    state::{
        CapabilityCallSignature, CheckpointKind, CheckpointPayloadError,
        DeferredCompactionWatermark, LoopExecutionState, RecoveryAttemptClass,
        RecoveryStrategyState, RepeatedCallWarningPhase, RepeatedCallWarningState,
    },
    test_support::{
        LoopExecutionStateBuilder, MockAgentLoopDriverHost, ScenarioScript, capability_id,
        test_run_context,
    },
};
use ironclaw_turns::{LoopExit, LoopFailureKind, run_profile::LoopRunInfoPort};
use serde_json::json;

#[test]
fn state_serializes_round_trips() {
    let signature =
        CapabilityCallSignature::from_call(capability_id("demo.echo"), &json!({ "x": 1 }))
            .expect("signature should build");
    let state = LoopExecutionStateBuilder::new()
        .iteration(7)
        .push_call_signature(signature)
        .push_failure_kind(LoopFailureKind::PolicyDenied)
        .recovery_attempts(2)
        .build();

    let encoded = serde_json::to_vec(&state).expect("state should serialize");
    let decoded: LoopExecutionState =
        serde_json::from_slice(&encoded).expect("state should deserialize");

    assert_eq!(decoded, state);
}

#[test]
fn state_serializes_round_trips_with_last_deferred_compaction_watermark() {
    let context = test_run_context("compaction-watermark-round-trip");
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
        through_seq: 42,
        prompt_fingerprint: 7_777,
    });

    let encoded = serde_json::to_vec(&state).expect("state should serialize");
    let decoded: LoopExecutionState =
        serde_json::from_slice(&encoded).expect("state should deserialize");

    assert_eq!(decoded.compaction_state, state.compaction_state);
    assert_eq!(
        decoded.compaction_state.last_deferred,
        Some(DeferredCompactionWatermark {
            through_seq: 42,
            prompt_fingerprint: 7_777,
        })
    );
}

#[test]
fn model_invalid_output_recovery_attempts_survive_checkpoint_reload() {
    let context = test_run_context("model-invalid-output-recovery-round-trip");
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.recovery_state =
        RecoveryStrategyState::with_attempts_for(RecoveryAttemptClass::ModelInvalidOutput, 2);

    let payload = serde_json::to_vec(&state).expect("state should serialize");
    let restored =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeModel)
            .expect("checkpoint payload should reload");

    assert_eq!(restored.recovery_state, state.recovery_state);
    assert_eq!(
        restored
            .recovery_state
            .attempts_for(RecoveryAttemptClass::ModelInvalidOutput),
        2
    );
}

#[test]
fn from_checkpoint_payload_rejects_non_state_payload() {
    let payload = serde_json::to_vec(&json!({
        "schema_id": "wrong",
        "payload": {}
    }))
    .expect("json should encode");

    let error = LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeModel)
        .expect_err("outer-envelope payload should not deserialize as state");

    assert!(matches!(
        error,
        CheckpointPayloadError::InvalidField {
            field: "payload",
            ..
        }
    ));
}

#[test]
fn recent_call_signatures_survive_serialization() {
    let context = test_run_context("signature-round-trip");
    let mut state = LoopExecutionState::initial_for_run(&context);
    for index in 0..5 {
        state.recent_call_signatures.push(
            CapabilityCallSignature::from_call(
                capability_id("demo.echo"),
                &json!({ "index": index }),
            )
            .expect("signature should build"),
        );
    }

    let encoded = serde_json::to_vec(&state).expect("state should serialize");
    let decoded: LoopExecutionState =
        serde_json::from_slice(&encoded).expect("state should deserialize");

    assert_eq!(
        decoded
            .recent_call_signatures
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        state
            .recent_call_signatures
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    );
}

#[test]
fn repeated_call_warning_state_survives_serialization() {
    let context = test_run_context("repeated-call-warning-round-trip");
    let mut state = LoopExecutionState::initial_for_run(&context);
    let signature = CapabilityCallSignature::from_call(
        capability_id("demo.echo"),
        &json!({ "query": "repeat" }),
    )
    .expect("signature should build");
    state.stop_state.repeated_call_warning =
        Some(RepeatedCallWarningState::rendered(signature.clone()));

    let encoded = serde_json::to_vec(&state).expect("state should serialize");
    let decoded: LoopExecutionState =
        serde_json::from_slice(&encoded).expect("state should deserialize");

    let warning = decoded
        .stop_state
        .repeated_call_warning
        .expect("warning should round-trip");
    assert_eq!(warning.signature, signature);
    assert_eq!(warning.phase, RepeatedCallWarningPhase::Rendered);
}

#[test]
fn old_stop_state_without_repeated_call_warning_loads_with_default() {
    let context = test_run_context("old-stop-state-repeated-warning-default");
    let state = LoopExecutionState::initial_for_run(&context);
    let mut value = serde_json::to_value(&state).expect("state should serialize");
    value
        .get_mut("stop_state")
        .and_then(serde_json::Value::as_object_mut)
        .expect("stop state object")
        .remove("repeated_call_warning");

    let decoded: LoopExecutionState =
        serde_json::from_value(value).expect("old state should deserialize");

    assert!(decoded.stop_state.repeated_call_warning.is_none());
}

#[test]
fn args_hash_jcs_stable() {
    let pretty = json!({
        "b": 2,
        "a": {
            "z": [3, 2, 1],
            "m": "x"
        }
    });
    let reordered = json!({
        "a": {
            "m": "x",
            "z": [3, 2, 1]
        },
        "b": 2
    });

    let first = CapabilityCallSignature::from_call(capability_id("demo.echo"), &pretty)
        .expect("signature should build");
    let second = CapabilityCallSignature::from_call(capability_id("demo.echo"), &reordered)
        .expect("signature should build");

    assert_eq!(first.args_hash, second.args_hash);
}

#[tokio::test]
async fn checkpoint_payload_reload_continues_through_executor() {
    let (host, checkpoints) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("after reload"))
        .build();
    let initial = LoopExecutionState::initial_for_run(host.run_context());
    let payload = serde_json::to_vec(&initial).expect("state should serialize");
    let reloaded =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeSideEffect)
            .expect("checkpoint payload should reload");

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, reloaded)
        .await
        .expect("loop execution should succeed after reload");

    assert!(matches!(exit, LoopExit::Completed(_)));
    checkpoints.assert_sequence(&[(CheckpointKind::BeforeModel, 0), (CheckpointKind::Final, 0)]);
}
