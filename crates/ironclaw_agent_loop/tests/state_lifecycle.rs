use ironclaw_agent_loop::{
    state::{CapabilityCallSignature, CheckpointKind, CheckpointPayloadError, LoopExecutionState},
    test_support::{LoopExecutionStateBuilder, capability_id, test_run_context},
};
use ironclaw_turns::LoopFailureKind;
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
