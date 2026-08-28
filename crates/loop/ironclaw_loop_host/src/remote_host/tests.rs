use super::protocol::*;
use ironclaw_loop_contracts::*;

#[test]
fn loop_worker_wire_rejects_oversized_frames_before_transport() {
    let outcome = LoopWorkerOutcome::Failed(LoopWorkerFailure {
        kind: "oversized".to_string(),
        detail: Some("x".repeat(LOOP_WORKER_MAX_FRAME_BYTES)),
    });
    let error = encode(&WorkerFrame::Outcome(outcome)).expect_err("oversized frame must fail");
    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

#[test]
fn private_surface_wire_round_trips_host_assigned_runtime_kind() {
    let descriptor = CapabilityDescriptorView {
        capability_id: ironclaw_host_api::ids::CapabilityId::new("builtin.shell")
            .expect("capability id"),
        provider: None,
        runtime: ironclaw_host_api::runtime::RuntimeKind::FirstParty,
        safe_name: "builtin.shell".to_string(),
        safe_description: "Run a command".to_string(),
        description_trust: Default::default(),
        parameters_schema: serde_json::json!({"type": "object"}),
    };
    let wire = WireVisibleCapabilitySurface::from(VisibleCapabilitySurface {
        version: CapabilitySurfaceVersion::new("surface-v1").expect("surface version"),
        descriptors: vec![descriptor],
        callable_capability_ids: None,
    });
    let encoded = serde_json::to_vec(&wire).expect("surface serializes");
    let decoded: WireVisibleCapabilitySurface =
        serde_json::from_slice(&encoded).expect("trusted surface deserializes");
    let restored = VisibleCapabilitySurface::from(decoded);
    assert_eq!(
        restored.descriptors[0].runtime,
        ironclaw_host_api::runtime::RuntimeKind::FirstParty
    );
}

#[test]
fn private_context_wire_preserves_the_complete_empty_bundle_shape() {
    let original = LoopContextBundle::default();
    let wire = WireLoopContextBundle::from(original.clone());
    let encoded = serde_json::to_vec(&wire).expect("wire context serializes");
    let decoded: WireLoopContextBundle =
        serde_json::from_slice(&encoded).expect("wire context deserializes");
    assert_eq!(LoopContextBundle::from(decoded), original);
}

#[test]
fn private_checkpoint_wire_revalidates_redacted_payload_bytes() {
    let original = LoadedCheckpointPayload {
        kind: LoopCheckpointKind::BeforeModel,
        schema_id: CheckpointSchemaId::new("canonical-loop-state").expect("schema id"),
        schema_version: ironclaw_host_api::turn::RunProfileVersion::new(1),
        payload: RedactedCheckpointPayload::new(br#"{"iteration":1}"#.to_vec())
            .expect("bounded payload"),
    };
    let wire = WireLoadedCheckpointPayload::from(original.clone());
    let encoded = serde_json::to_vec(&wire).expect("wire checkpoint serializes");
    let decoded: WireLoadedCheckpointPayload =
        serde_json::from_slice(&encoded).expect("wire checkpoint deserializes");
    let restored = LoadedCheckpointPayload::try_from(decoded).expect("payload revalidates");
    assert_eq!(restored, original);
}
