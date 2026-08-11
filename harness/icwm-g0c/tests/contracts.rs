use icwm_g0c_harness::{
    AdmissionTrigger, ArtifactReport, CandidateAdapter, CandidateId, ControlAdapter, EffectIntent,
    EffectKind, Failpoint, Harness, HarnessError, MatrixIdentifierKind, MatrixInput, RestartReason,
    ScriptedResponse, StableId, StatefulResponder, admit_fixture_message,
    validate_matrix_identifier, validate_matrix_identifier_bytes,
};
use std::collections::BTreeMap;

fn request(id: &str) -> EffectIntent {
    EffectIntent::new(
        EffectKind::MatrixRequest {
            method: "POST".into(),
            path: "/_matrix/client/v3/keys/query".into(),
            body: br#"{"device_keys":{}}"#.to_vec(),
        },
        StableId::derive("scenario", &[id.as_bytes()]),
    )
}

#[test]
fn control_adapter_obeys_ordered_script_and_virtual_clock() {
    let expected = request("query-1");
    let mut harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![expected.clone()],
        vec![ScriptedResponse::matrix(
            200,
            br#"{"device_keys":{}}"#.to_vec(),
        )],
    );

    harness.clock_mut().advance_ms(25).unwrap();
    harness.drive(MatrixInput::Emit(expected.clone())).unwrap();
    harness.drive(MatrixInput::ConsumeNextResponse).unwrap();

    let report = harness.finish().unwrap();
    assert_eq!(report.effects.len(), 1);
    assert_eq!(report.effects[0].sequence, 0);
    assert_eq!(report.effects[0].at_ms, 25);
    assert_eq!(report.effects[0].intent, expected);
}

#[test]
fn unexpected_reordered_and_missing_effects_fail_closed() {
    let first = request("first");
    let second = request("second");

    let mut reordered = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![first.clone(), second.clone()],
        vec![],
    );
    let error = reordered.drive(MatrixInput::Emit(second)).unwrap_err();
    assert!(matches!(error, HarnessError::ReorderedEffect { .. }));

    let mut unexpected = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![],
        vec![],
    );
    let error = unexpected
        .drive(MatrixInput::Emit(first.clone()))
        .unwrap_err();
    assert!(matches!(error, HarnessError::UnexpectedEffect { .. }));

    let missing = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![first],
        vec![],
    );
    let report = missing.finish().unwrap();
    assert_eq!(
        report.disposition,
        icwm_g0c_harness::ResultDisposition::Failed
    );
    assert_eq!(
        report.disposition_reason.as_deref(),
        Some("MissingEffects { count: 1 }")
    );
}

#[test]
fn script_exhaustion_fails_closed() {
    let expected = request("query");
    let mut harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![expected.clone()],
        vec![ScriptedResponse::matrix(200, b"ok".to_vec())],
    );
    harness.drive(MatrixInput::Emit(expected)).unwrap();
    harness.drive(MatrixInput::ConsumeNextResponse).unwrap();
    assert!(matches!(
        harness.drive(MatrixInput::ConsumeNextResponse),
        Err(HarnessError::MissingScriptedResponse)
    ));
}

#[test]
fn terminal_completeness_failures_are_normalized_into_publishable_reports() {
    let expected = request("never-emitted");
    let harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![expected],
        vec![ScriptedResponse::matrix(200, b"unused".to_vec())],
    );
    let execution = harness.finish().unwrap();
    assert_eq!(
        execution.disposition,
        icwm_g0c_harness::ResultDisposition::Failed
    );
    let reason = execution.disposition_reason.as_deref().unwrap();
    assert!(reason.contains("MissingEffects { count: 1 }"));
    assert!(reason.contains("UnusedScriptedResponses { count: 1 }"));

    let mut artifact: ArtifactReport =
        serde_json::from_str(include_str!("../fixtures/CONTROL-RESULT.json")).unwrap();
    artifact.apply_execution(&execution).unwrap();
    assert_eq!(
        artifact.disposition,
        icwm_g0c_harness::ResultDisposition::Failed
    );
    artifact.validate_against_execution(&execution).unwrap();
}

#[test]
fn crash_and_restart_hooks_preserve_ledger_and_reset_volatile_state() {
    let expected = request("before-crash");
    let mut harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![expected.clone()],
        vec![],
    );
    harness.drive(MatrixInput::Emit(expected)).unwrap();

    let crash = harness.crash().unwrap();
    assert_eq!(crash.reached, None);
    harness.restart(RestartReason::CrashRecovery).unwrap();
    assert_eq!(harness.adapter().restart_count(), 1);
    assert_eq!(harness.finish().unwrap().effects.len(), 1);
}

#[test]
fn explicit_crash_does_not_credit_an_untraversed_armed_failpoint() {
    let required = Failpoint::CandidateBeforePrepare;
    let mut harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![],
        vec![],
    )
    .with_required_failpoints(vec![required])
    .unwrap();
    harness.arm_failpoint(required);
    assert_eq!(harness.crash().unwrap().reached, None);
    let report = harness.finish().unwrap();
    assert!(report.failpoints_reached.is_empty());
    assert_eq!(report.crashes.len(), 1);
    let reason = report.disposition_reason.unwrap();
    assert!(reason.contains("UnreachedFailpoints { count: 1 }"));
    assert!(reason.contains("ArmedFailpointUnreached"));

    let operation_id = StableId::derive("traversed-required-failpoint", &[]);
    let mut traversed = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![],
        vec![],
    )
    .with_required_failpoints(vec![required])
    .unwrap();
    traversed.arm_failpoint(required);
    assert!(matches!(
        traversed.drive(MatrixInput::BeginOperation { operation_id }),
        Err(HarnessError::InjectedFailpoint { failpoint }) if failpoint == required
    ));
    let report = traversed.finish().unwrap();
    assert_eq!(report.failpoints_reached, vec![required]);
    assert_eq!(report.crashes.len(), 1);
    assert!(
        !report
            .disposition_reason
            .as_deref()
            .unwrap()
            .contains("UnreachedFailpoints")
    );
}

#[test]
fn run_plan_cannot_be_applied_after_execution_starts() {
    let operation_id = StableId::derive("run-plan-before-execution", &[]);
    let mut harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![],
        vec![],
    );
    harness
        .drive(MatrixInput::BeginOperation { operation_id })
        .unwrap();
    assert!(matches!(
        harness.with_run_plan(icwm_g0c_harness::RunPlan::default()),
        Err(HarnessError::RunPlanAfterExecutionStarted)
    ));

    let mut restarted = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![],
        vec![],
    );
    restarted.restart(RestartReason::CleanRestart).unwrap();
    assert!(matches!(
        restarted.with_run_plan(icwm_g0c_harness::RunPlan::default()),
        Err(HarnessError::RunPlanAfterExecutionStarted)
    ));

    let operation_id = StableId::derive("required-after-execution", &[]);
    let mut started = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![],
        vec![],
    );
    started
        .drive(MatrixInput::BeginOperation { operation_id })
        .unwrap();
    assert!(matches!(
        started.with_required_failpoints(vec![]),
        Err(HarnessError::RequiredFailpointsAfterExecutionStarted)
    ));
}

#[test]
fn stable_ids_use_typed_length_delimited_material_not_serialized_json() {
    let left = StableId::derive("effect", &[b"ab".as_slice(), b"c".as_slice()]);
    let right = StableId::derive("effect", &[b"a".as_slice(), b"bc".as_slice()]);
    let other_domain = StableId::derive("candidate", &[b"ab".as_slice(), b"c".as_slice()]);
    assert_ne!(left, right);
    assert_ne!(left, other_domain);
    assert_eq!(left.as_str().len(), 64);
    assert_eq!(
        left.as_str(),
        "e8a72c93c99554ee0d3eba350c5128682b1440a2377df6e5afa5754fd6b8b84b"
    );
}

#[test]
fn virtual_clock_rejects_overflow() {
    let mut harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![],
        vec![],
    );
    harness.clock_mut().set_ms(u64::MAX);
    assert!(harness.clock_mut().advance_ms(1).is_err());
}

#[derive(Clone)]
struct BatchAdapter {
    id: CandidateId,
    batch: Vec<EffectIntent>,
}

impl CandidateAdapter for BatchAdapter {
    type Prepared = MatrixInput;
    type PreparedResponse = ScriptedResponse;

    fn candidate(&self) -> &CandidateId {
        &self.id
    }
    fn prepare(&mut self, input: MatrixInput) -> Result<Self::Prepared, HarnessError> {
        Ok(input)
    }
    fn commit(&mut self, _prepared: &Self::Prepared) -> Result<Vec<EffectIntent>, HarnessError> {
        Ok(self.batch.clone())
    }
    fn acknowledge(&mut self, _prepared: Self::Prepared) -> Result<(), HarnessError> {
        Ok(())
    }
    fn prepare_response(
        &mut self,
        response: ScriptedResponse,
    ) -> Result<Self::PreparedResponse, HarnessError> {
        Ok(response)
    }
    fn commit_response(
        &mut self,
        _response: &Self::PreparedResponse,
    ) -> Result<Vec<EffectIntent>, HarnessError> {
        Ok(vec![])
    }
    fn acknowledge_response(
        &mut self,
        _response: Self::PreparedResponse,
    ) -> Result<(), HarnessError> {
        Ok(())
    }
    fn crash(&mut self, _failpoint: Option<Failpoint>) -> Result<(), HarnessError> {
        Ok(())
    }
    fn restart(&mut self, _reason: RestartReason) -> Result<(), HarnessError> {
        Ok(())
    }
}

#[test]
fn invalid_effect_batch_is_rejected_atomically() {
    let first = request("first");
    let expected_second = request("expected-second");
    let wrong_second = request("wrong-second");
    let adapter = BatchAdapter {
        id: CandidateId::new("batch", "1"),
        batch: vec![first.clone(), wrong_second],
    };
    let mut harness = Harness::new(adapter, vec![first, expected_second], vec![]);
    assert!(matches!(
        harness.drive(MatrixInput::ConsumeNextResponse),
        Err(HarnessError::MissingScriptedResponse)
    ));
    assert!(matches!(
        harness.drive(MatrixInput::Sync {
            next_batch: "s".into(),
            body: vec![]
        }),
        Err(HarnessError::ReorderedEffect { .. })
    ));
    let report = harness.finish().unwrap();
    assert_eq!(
        report.disposition,
        icwm_g0c_harness::ResultDisposition::Failed
    );
    assert!(
        report
            .disposition_reason
            .unwrap()
            .contains("MissingEffects { count: 2 }")
    );
}

#[test]
fn boundary_failpoints_execute_before_and_after_effect() {
    let effect = EffectIntent::new(
        EffectKind::CryptoStoreWrite {
            scope: "acct".into(),
            record_type: "session".into(),
            bytes: vec![1],
        },
        StableId::derive("store", &[b"one"]),
    );
    let mut before = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![effect.clone()],
        vec![],
    );
    before.arm_failpoint(Failpoint::LedgerBeforeCryptoStoreAppend);
    assert!(matches!(
        before.drive(MatrixInput::Emit(effect.clone())),
        Err(HarnessError::InjectedFailpoint {
            failpoint: Failpoint::LedgerBeforeCryptoStoreAppend
        })
    ));
    let report = before.finish().unwrap();
    assert_eq!(
        report.disposition,
        icwm_g0c_harness::ResultDisposition::Failed
    );
    assert!(
        report
            .disposition_reason
            .unwrap()
            .contains("MissingEffects { count: 1 }")
    );

    let mut after = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![effect.clone()],
        vec![],
    );
    after.arm_failpoint(Failpoint::LedgerAfterCryptoStoreAppend);
    assert!(matches!(
        after.drive(MatrixInput::Emit(effect)),
        Err(HarnessError::InjectedFailpoint {
            failpoint: Failpoint::LedgerAfterCryptoStoreAppend
        })
    ));
    assert_eq!(after.finish().unwrap().effects.len(), 1);
}

#[test]
fn response_delivery_loss_is_executable() {
    let mut harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![],
        vec![
            ScriptedResponse::matrix(200, b"ok".to_vec())
                .with_delivery(icwm_g0c_harness::ResponseDelivery::LostBeforeCandidate),
        ],
    );
    assert!(matches!(
        harness.drive(MatrixInput::ConsumeNextResponse),
        Err(HarnessError::ResponseLostBeforeCandidate)
    ));
    assert!(harness.adapter().observed_response_bodies().is_empty());
    assert_eq!(harness.adapter().restart_count(), 0);
    harness
        .drive(MatrixInput::BeginOperation {
            operation_id: StableId::derive("live-after-response-loss", &[b"before"]),
        })
        .unwrap();
    assert_eq!(harness.adapter().acknowledged_inputs(), 1);
    let report = harness.finish().unwrap();
    assert!(report.crashes.is_empty());
    assert!(report.failpoints_reached.is_empty());
    assert_eq!(report.response_losses.len(), 1);
    assert_eq!(
        report.disposition,
        icwm_g0c_harness::ResultDisposition::Failed
    );
}

#[test]
fn partial_response_exposes_only_the_scripted_prefix() {
    let mut harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![],
        vec![ScriptedResponse::matrix(200, b"abcdef".to_vec()).partial(3)],
    );
    harness.drive(MatrixInput::ConsumeNextResponse).unwrap();
    assert_eq!(
        harness.adapter().observed_response_bodies(),
        &[b"abc".to_vec()]
    );
    harness.finish().unwrap();
}

#[test]
fn lost_after_adapter_handling_runs_adapter_then_reports_uncertainty() {
    let mut harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![],
        vec![
            ScriptedResponse::matrix(200, b"committed".to_vec())
                .with_delivery(icwm_g0c_harness::ResponseDelivery::LostAfterAdapterHandling),
        ],
    );
    assert!(matches!(
        harness.drive(MatrixInput::ConsumeNextResponse),
        Err(HarnessError::ResponseLostAfterAdapterHandling)
    ));
    assert_eq!(
        harness.adapter().observed_response_bodies(),
        &[b"committed".to_vec()]
    );
    assert_eq!(harness.adapter().acknowledged_responses(), 0);
    assert_eq!(harness.adapter().restart_count(), 0);
    harness
        .drive(MatrixInput::BeginOperation {
            operation_id: StableId::derive("live-after-response-loss", &[b"after"]),
        })
        .unwrap();
    assert_eq!(harness.adapter().acknowledged_inputs(), 1);
    let report = harness.finish().unwrap();
    assert!(report.crashes.is_empty());
    assert!(report.failpoints_reached.is_empty());
    assert_eq!(report.response_losses.len(), 1);
    assert_eq!(
        report.disposition,
        icwm_g0c_harness::ResultDisposition::Uncertain
    );
}

#[test]
fn policy_records_are_predeclared_structured_and_execution_bound() {
    let record = icwm_g0c_harness::PolicyRecord {
        scope: "candidate:control/scenario:sync".into(),
        reason: "known bounded fixture limitation".into(),
        evidence: "sha256:0123456789abcdef".into(),
        reviewer_identity: "reviewer@example.org".into(),
    };
    let harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![],
        vec![],
    )
    .with_run_plan(icwm_g0c_harness::RunPlan {
        expected_failures: vec![record.clone()],
        waivers: vec![record.clone()],
        blacklists: vec![record.clone()],
        ..Default::default()
    })
    .unwrap();
    let execution = harness.finish().unwrap();
    assert_eq!(execution.expected_failures, vec![record.clone()]);
    let mut artifact: ArtifactReport =
        serde_json::from_str(include_str!("../fixtures/CONTROL-RESULT.json")).unwrap();
    artifact.apply_execution(&execution).unwrap();
    artifact.validate_against_execution(&execution).unwrap();
    artifact.waivers.clear();
    assert!(matches!(
        artifact.validate_against_execution(&execution),
        Err(HarnessError::ArtifactExecutionMismatch)
    ));
}

#[test]
fn cancellation_releases_a_real_active_operation_and_rejects_replay() {
    let operation_id = StableId::derive("operation", &[b"sync-1"]);
    let cancellation = EffectIntent::new(
        EffectKind::Cancellation {
            operation_id: operation_id.clone(),
        },
        StableId::derive("cancellation", &[operation_id.as_str().as_bytes()]),
    );
    let step = EffectIntent::new(
        EffectKind::Observation {
            name: "operation_step".into(),
            value: "poll".into(),
        },
        StableId::derive(
            "operation-step",
            &[operation_id.as_str().as_bytes(), b"poll"],
        ),
    );
    let mut harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![step, cancellation],
        vec![],
    );
    harness
        .drive(MatrixInput::BeginOperation {
            operation_id: operation_id.clone(),
        })
        .unwrap();
    harness
        .drive(MatrixInput::StepOperation {
            operation_id: operation_id.clone(),
            step: "poll".into(),
        })
        .unwrap();
    harness
        .drive(MatrixInput::Cancel {
            operation_id: operation_id.clone(),
        })
        .unwrap();
    assert!(harness.adapter().is_cancelled(&operation_id));
    assert!(matches!(
        harness.drive(MatrixInput::StepOperation {
            operation_id,
            step: "late".into()
        }),
        Err(HarnessError::OperationNotActive { .. })
    ));
}

#[test]
fn identifier_admission_rejects_raw_invalid_utf8_before_string_allocation() {
    assert!(matches!(
        validate_matrix_identifier_bytes(&[0xff]),
        Err(HarnessError::InvalidIdentifier)
    ));
    assert!(matches!(
        validate_matrix_identifier_bytes(b"room\n"),
        Err(HarnessError::InvalidIdentifier)
    ));
}

#[test]
fn candidate_commit_failpoint_is_bound_to_an_explicit_candidate_boundary() {
    let boundary = request("candidate-commit");
    let mut harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![boundary.clone()],
        vec![],
    );
    harness.arm_failpoint(Failpoint::CandidateAfterCommitBeforeAcknowledge);
    assert!(matches!(
        harness.drive(MatrixInput::Emit(boundary)),
        Err(HarnessError::InjectedFailpoint {
            failpoint: Failpoint::CandidateAfterCommitBeforeAcknowledge
        })
    ));
    assert_eq!(harness.adapter().acknowledged_inputs(), 0);
    assert_eq!(harness.finish().unwrap().effects.len(), 1);
}

#[test]
fn responder_releases_sync_cross_signing_to_device_and_captures_requests() {
    let mut responder = StatefulResponder::default();
    responder.queue_sync_release(b"sync-one".to_vec());
    responder.set_cross_signing_keys("@a:example".into(), b"master-key".to_vec());
    responder.send_to_device("DEVICE".into(), b"room-key".to_vec());
    let mut vector = icwm_g0c_harness::RequestVector {
        schema_version: "icwm.g0c.request-vector.v1".into(),
        vector_id: StableId::derive("request-vector", &[b"keys-query"]),
        purpose: "keys_query".into(),
        method: "POST".into(),
        path: "/_matrix/client/v3/keys/query".into(),
        credential_free_body: br#"{"device_keys":{}}"#.to_vec(),
    };
    vector.vector_id = vector.derived_id();
    vector.validate_identity().unwrap();
    responder.capture_request(vector.clone());
    assert_eq!(responder.release_sync(), Some(b"sync-one".to_vec()));
    assert_eq!(responder.release_sync(), None);
    assert_eq!(
        responder.query_cross_signing_keys("@a:example"),
        Some(b"master-key".as_slice())
    );
    assert_eq!(
        responder.receive_to_device("DEVICE"),
        Some(b"room-key".to_vec())
    );
    assert_eq!(responder.receive_to_device("DEVICE"), None);
    assert_eq!(responder.captured_requests(), &[vector]);

    let mut request_driven = StatefulResponder::default();
    request_driven.queue_sync_release(b"sync-two".to_vec());
    let mut sync = icwm_g0c_harness::RequestVector {
        schema_version: "icwm.g0c.request-vector.v1".into(),
        vector_id: StableId::derive("placeholder", &[]),
        purpose: "sync".into(),
        method: "GET".into(),
        path: "/_matrix/client/v3/sync".into(),
        credential_free_body: vec![],
    };
    sync.vector_id = sync.derived_id();
    assert_eq!(request_driven.respond(&sync).unwrap(), b"sync-two".to_vec());
    assert_eq!(request_driven.captured_requests(), &[sync]);
}

#[test]
fn responder_correlates_key_responses_to_requested_identity_and_algorithm() {
    fn vector(purpose: &str, body: &[u8]) -> icwm_g0c_harness::RequestVector {
        let mut vector = icwm_g0c_harness::RequestVector {
            schema_version: "icwm.g0c.request-vector.v1".into(),
            vector_id: StableId::derive("placeholder", &[]),
            purpose: purpose.into(),
            method: "POST".into(),
            path: "/_matrix/client/v3/keys/test".into(),
            credential_free_body: body.to_vec(),
        };
        vector.vector_id = vector.derived_id();
        vector
    }

    let mut responder = StatefulResponder::default();
    responder.set_device_key(
        "@alice:example".into(),
        "ALICE".into(),
        b"alice-device".to_vec(),
    );
    responder.set_device_key("@bob:example".into(), "BOB".into(), b"bob-device".to_vec());
    responder.set_cross_signing_keys("@alice:example".into(), b"alice-signing".to_vec());
    responder.set_cross_signing_keys("@bob:example".into(), b"bob-signing".to_vec());
    responder.add_one_time_key(
        "@alice:example".into(),
        "ALICE".into(),
        "signed_curve25519".into(),
        b"alice-otk".to_vec(),
    );
    responder.add_one_time_key(
        "@bob:example".into(),
        "BOB".into(),
        "curve25519".into(),
        b"bob-otk".to_vec(),
    );

    assert_eq!(
        responder
            .respond(&vector(
                "keys_query",
                br#"{"device_keys":{"@bob:example":["BOB"]}}"#,
            ))
            .unwrap(),
        b"bob-device"
    );
    assert!(matches!(
        responder.respond(&vector(
            "keys_query",
            br#"{"device_keys":{"@bob:example":["NOT-BOB"]}}"#,
        )),
        Err(HarnessError::ResponderUnavailable)
    ));
    assert_eq!(
        responder
            .respond(&vector(
                "keys_claim",
                br#"{"one_time_keys":{"@bob:example":{"BOB":"curve25519"}}}"#,
            ))
            .unwrap(),
        b"bob-otk"
    );
    assert!(matches!(
        responder.respond(&vector(
            "keys_claim",
            br#"{"one_time_keys":{"@alice:example":{"ALICE":"curve25519"}}}"#,
        )),
        Err(HarnessError::ResponderUnavailable)
    ));
    assert_eq!(
        responder
            .respond(&vector(
                "cross_signing",
                br#"{"master_key":{"user_id":"@bob:example"}}"#,
            ))
            .unwrap(),
        b"bob-signing"
    );
    assert!(matches!(
        responder.respond(&vector(
            "keys_query",
            br#"{"device_keys":{"@alice:example":[],"@bob:example":[]}}"#,
        )),
        Err(HarnessError::ResponderRequestInvalid)
    ));
}

#[test]
fn artifact_report_denies_unknown_policy_fields_and_preserves_known_ones() {
    let source = include_str!("../fixtures/CONTROL-RESULT.json");
    let report: ArtifactReport = serde_json::from_str(source).unwrap();
    assert_eq!(
        report.capabilities.get("ordered_effect_ledger"),
        Some(&true)
    );
    assert!(report.expected_failures.is_empty());
    assert!(report.waivers.is_empty());
    assert!(report.blacklists.is_empty());

    let mut value: serde_json::Value = serde_json::from_str(source).unwrap();
    value["unknown_policy"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ArtifactReport>(value).is_err());
}

#[test]
fn published_control_scenario_deserializes_and_drives_the_control_adapter() {
    let scenario: icwm_g0c_harness::Scenario =
        serde_json::from_str(include_str!("../fixtures/CONTROL-SCENARIO.json")).unwrap();
    scenario.validate_identity().unwrap();
    let mut harness = Harness::from_scenario(
        ControlAdapter::new(CandidateId::new("control", "1")),
        &scenario,
        vec![ScriptedResponse::matrix(200, b"{}".to_vec())],
    )
    .unwrap();
    let capabilities = BTreeMap::from([
        ("ordered_effect_ledger".into(), true),
        ("scripted_response_consumption".into(), true),
        ("virtual_clock".into(), true),
    ]);
    let capability_dispositions: BTreeMap<String, icwm_g0c_harness::ResultDisposition> = [
        "sync",
        "keys_query",
        "keys_claim",
        "to_device",
        "room_send",
        "keys_upload",
        "device_list",
        "room_keys_backup",
        "cross_signing",
        "signatures_upload",
        "verification",
        "keys_changes",
    ]
    .into_iter()
    .map(|name| {
        (
            name.to_owned(),
            icwm_g0c_harness::ResultDisposition::NotApplicable,
        )
    })
    .collect();
    let capability_disposition_reasons = capability_dispositions
        .keys()
        .map(|name| {
            (
                name.clone(),
                "Outside the candidate-neutral control adapter topology.".to_owned(),
            )
        })
        .collect();
    harness = harness
        .with_run_plan(icwm_g0c_harness::RunPlan {
            disposition_reason: Some("The control adapter exercises only candidate-neutral harness mechanics; Matrix transport and crypto capabilities are outside this control topology.".into()),
            capabilities,
            capability_dispositions,
            capability_disposition_reasons,
            ..Default::default()
        })
        .unwrap();
    harness.clock_mut().advance_ms(25).unwrap();
    for input in scenario.inputs {
        harness.drive(input).unwrap();
    }
    let execution = harness.finish().unwrap();
    assert_eq!(
        execution.disposition,
        icwm_g0c_harness::ResultDisposition::Supported
    );
    assert_eq!(execution.effects.len(), 1);
    assert_eq!(
        execution.effects[0].intent.effect_id.as_str(),
        "ea99c992173e0369b9f4fa5a764acaf1e68e3028a540d46566e8b134c89e2cad"
    );
    let artifact: ArtifactReport =
        serde_json::from_str(include_str!("../fixtures/CONTROL-RESULT.json")).unwrap();
    assert_eq!(artifact.disposition, execution.disposition);
    artifact.validate_against_execution(&execution).unwrap();
}

#[test]
fn request_vector_corpus_binds_every_id_to_exact_request_bytes() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/REQUEST-VECTORS-v1.json")).unwrap();
    let vectors = corpus["vectors"].as_array().unwrap();
    assert_eq!(vectors.len(), 12);
    let mut ids = Vec::new();
    for value in vectors {
        let vector: icwm_g0c_harness::RequestVector =
            serde_json::from_value(value.clone()).unwrap();
        vector.validate_identity().unwrap();
        ids.push(vector.vector_id.as_str().as_bytes().to_vec());
    }
    let components: Vec<&[u8]> = ids.iter().map(Vec::as_slice).collect();
    assert_eq!(
        StableId::derive("request-vector-corpus", &components).as_str(),
        corpus["corpus_id"].as_str().unwrap()
    );
}

#[test]
fn artifact_reports_are_derived_and_checked_for_all_dispositions() {
    for disposition in [
        icwm_g0c_harness::ResultDisposition::Supported,
        icwm_g0c_harness::ResultDisposition::Uncertain,
        icwm_g0c_harness::ResultDisposition::Infeasible,
        icwm_g0c_harness::ResultDisposition::NotApplicable,
    ] {
        let effect = request("all-dispositions");
        let plan = match disposition {
            icwm_g0c_harness::ResultDisposition::Infeasible
            | icwm_g0c_harness::ResultDisposition::NotApplicable => icwm_g0c_harness::RunPlan {
                predeclared_disposition: Some(disposition),
                disposition_reason: Some("predeclared topology constraint".into()),
                ..Default::default()
            },
            _ => icwm_g0c_harness::RunPlan::default(),
        };
        let mut harness = Harness::new(
            ControlAdapter::new(CandidateId::new("control", "1")),
            vec![effect.clone()],
            vec![],
        )
        .with_run_plan(plan)
        .unwrap();
        if disposition == icwm_g0c_harness::ResultDisposition::Uncertain {
            harness.arm_failpoint(Failpoint::CandidateAfterCommitBeforeAcknowledge);
            assert!(harness.drive(MatrixInput::Emit(effect)).is_err());
        } else {
            harness.drive(MatrixInput::Emit(effect)).unwrap();
        }
        let execution = harness.finish().unwrap();
        assert_eq!(execution.disposition, disposition);
        let mut artifact: ArtifactReport =
            serde_json::from_str(include_str!("../fixtures/CONTROL-RESULT.json")).unwrap();
        artifact.apply_execution(&execution).unwrap();
        artifact.validate_against_execution(&execution).unwrap();
        let mut tampered = artifact.clone();
        tampered.effects[0].semantic_digest = "0".repeat(64);
        assert!(matches!(
            tampered.validate_against_execution(&execution),
            Err(HarnessError::ArtifactExecutionMismatch)
        ));
        for field in ["name", "version"] {
            let mut tampered = artifact.clone();
            if field == "name" {
                tampered.candidate.name.push_str("-wrong");
            } else {
                tampered.candidate.version.push_str("-wrong");
            }
            assert!(matches!(
                tampered.validate_against_execution(&execution),
                Err(HarnessError::ArtifactExecutionMismatch)
            ));
        }
    }
}

#[test]
fn artifact_report_binds_reached_failpoints() {
    let effect = request("reached-failpoint");
    let mut harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![effect.clone()],
        vec![],
    );
    harness.arm_failpoint(Failpoint::ProcessAfterEffectAppend);
    assert!(harness.drive(MatrixInput::Emit(effect)).is_err());
    let execution = harness.finish().unwrap();
    let mut artifact: ArtifactReport =
        serde_json::from_str(include_str!("../fixtures/CONTROL-RESULT.json")).unwrap();
    artifact.apply_execution(&execution).unwrap();
    artifact.validate_against_execution(&execution).unwrap();
    assert_eq!(
        artifact.failpoints_reached,
        vec!["process_after_effect_append"]
    );
}

#[test]
fn identifier_and_admission_fixture_executes_every_case() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/IDENTIFIER-FIXTURE.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 23);
    let mut oracle_accepts_harness_rejects = false;
    let mut harness_accepts_oracle_rejects = false;
    for case in cases {
        let kind: MatrixIdentifierKind =
            serde_json::from_value(case["identifier_kind"].clone()).unwrap();
        let trigger: AdmissionTrigger = serde_json::from_value(case["trigger"].clone()).unwrap();
        let oracle_expected = case["oracle_expected"].as_str().unwrap();
        let identifier_expected = case["identifier_expected"].as_str().unwrap();
        let admission_expected = case["admission_expected"].as_str().unwrap();
        let value = if let Some(value) = case["value"].as_str() {
            value.as_bytes().to_vec()
        } else {
            let size = case["utf8_bytes"].as_u64().map(|v| v as usize);
            match (case["value_pattern"].as_str().unwrap(), kind, size) {
                (
                    "typed_ascii_exact_limit" | "typed_ascii_over_limit",
                    MatrixIdentifierKind::UserId,
                    Some(size),
                ) => format!("@{}:x", "a".repeat(size - 3)).into_bytes(),
                (
                    "typed_ascii_exact_limit" | "typed_ascii_over_limit",
                    MatrixIdentifierKind::RoomId,
                    Some(size),
                ) => format!("!{}:x", "a".repeat(size - 3)).into_bytes(),
                ("opaque_ascii_exact_limit" | "opaque_ascii_over_limit", _, Some(size)) => {
                    vec![b'a'; size]
                }
                ("invalid_utf8_ff", _, None) => vec![0xff],
                other => panic!("unknown fixture pattern: {other:?}"),
            }
        };
        let actor = case["actor"].as_str().map(str::as_bytes);
        let identifier_accepted =
            icwm_g0c_harness::validate_typed_matrix_identifier_bytes(kind, &value).is_ok();
        assert_eq!(
            identifier_accepted,
            identifier_expected == "accepted",
            "harness identifier result for fixture case {case:?}"
        );
        let admission_accepted =
            admit_fixture_message(actor, kind, &value, trigger).unwrap_or(false);
        assert_eq!(
            admission_accepted,
            admission_expected == "accepted",
            "message admission result for fixture case {case:?}"
        );

        let oracle_accepted = std::str::from_utf8(&value).ok().map(|value| match kind {
            MatrixIdentifierKind::UserId => ruma_common::UserId::parse(value).is_ok(),
            MatrixIdentifierKind::RoomId => ruma_common::RoomId::parse(value).is_ok(),
            MatrixIdentifierKind::DeviceId => {
                let _: ruma_common::OwnedDeviceId = value.into();
                true
            }
            MatrixIdentifierKind::SessionId => <&ruma_common::SessionId>::try_from(value).is_ok(),
        });
        match oracle_expected {
            "accepted" => assert_eq!(oracle_accepted, Some(true), "oracle case {case:?}"),
            "rejected" => assert_eq!(oracle_accepted, Some(false), "oracle case {case:?}"),
            "not_applicable" => assert_eq!(oracle_accepted, None, "oracle case {case:?}"),
            other => panic!("unknown oracle expectation {other:?}"),
        }
        if let Some(oracle_accepted) = oracle_accepted {
            oracle_accepts_harness_rejects |= oracle_accepted && !identifier_accepted;
            harness_accepts_oracle_rejects |= !oracle_accepted && identifier_accepted;
        }
    }
    assert!(oracle_accepts_harness_rejects);
    assert!(harness_accepts_oracle_rejects);
}

#[test]
fn stateful_responder_claims_are_consumptive_and_identifier_bound_is_bytes() {
    let mut responder = StatefulResponder::default();
    responder.set_device_keys("@a:example".into(), b"keys".to_vec());
    responder.add_one_time_key(
        "@a:example".into(),
        "D".into(),
        "signed_curve25519".into(),
        b"otk".to_vec(),
    );
    assert_eq!(
        responder.query_device_keys("@a:example"),
        Some(b"keys".as_slice())
    );
    assert_eq!(
        responder.claim_one_time_key("@a:example", "D", "signed_curve25519"),
        Some(b"otk".to_vec())
    );
    assert_eq!(
        responder.claim_one_time_key("@a:example", "D", "signed_curve25519"),
        None
    );
    assert!(validate_matrix_identifier(&"a".repeat(255)).is_ok());
    assert!(matches!(
        validate_matrix_identifier(&"é".repeat(128)),
        Err(HarnessError::IdentifierTooLong { bytes: 256 })
    ));
    assert!(matches!(
        validate_matrix_identifier(""),
        Err(HarnessError::InvalidIdentifier)
    ));
    assert!(matches!(
        validate_matrix_identifier("room\n"),
        Err(HarnessError::InvalidIdentifier)
    ));
}

#[test]
fn published_control_result_round_trips_through_report_model() {
    let source = include_str!("../fixtures/CONTROL-RESULT.json");
    let report: ArtifactReport = serde_json::from_str(source).unwrap();
    let serialized = serde_json::to_string(&report).unwrap();
    let reparsed: ArtifactReport = serde_json::from_str(&serialized).unwrap();
    assert_eq!(report, reparsed);
    assert_eq!(report.schema_version, "icwm.g0c.harness-result.v1");
}

#[test]
fn capability_not_applicable_requires_a_capability_scoped_reason() {
    let plan = icwm_g0c_harness::RunPlan {
        capability_dispositions: BTreeMap::from([(
            "sync".into(),
            icwm_g0c_harness::ResultDisposition::NotApplicable,
        )]),
        ..Default::default()
    };
    assert!(matches!(
        Harness::new(
            ControlAdapter::new(CandidateId::new("control", "1")),
            vec![],
            vec![]
        )
        .with_run_plan(plan),
        Err(HarnessError::MissingCapabilityDispositionReason { capability })
            if capability == "sync"
    ));
}

#[test]
fn scenario_identity_and_required_failpoints_fail_closed() {
    let mut scenario: icwm_g0c_harness::Scenario =
        serde_json::from_str(include_str!("../fixtures/CONTROL-SCENARIO.json")).unwrap();
    scenario.validate_identity().unwrap();
    let mut tampered = scenario.clone();
    tampered.name.push_str("-tampered");
    assert!(matches!(
        tampered.validate_identity(),
        Err(HarnessError::ScenarioIdentityMismatch)
    ));

    scenario.failpoints = vec![Failpoint::CandidateBeforePrepare];
    scenario.scenario_id = scenario.derived_id().unwrap();
    let harness = Harness::from_scenario(
        ControlAdapter::new(CandidateId::new("control", "1")),
        &scenario,
        vec![ScriptedResponse::matrix(200, b"{}".to_vec())],
    )
    .unwrap();
    let report = harness.finish().unwrap();
    assert!(
        report
            .disposition_reason
            .as_deref()
            .unwrap()
            .contains("UnreachedFailpoints { count: 1 }")
    );
}

#[test]
fn armed_response_failpoints_execute_and_publish_crashes() {
    for failpoint in [
        Failpoint::ResponseBeforePrepare,
        Failpoint::ResponseAfterCommitBeforeAcknowledge,
    ] {
        let mut harness = Harness::new(
            ControlAdapter::new(CandidateId::new("control", "1")),
            vec![],
            vec![ScriptedResponse::matrix(200, b"response".to_vec())],
        );
        harness.arm_failpoint(failpoint);
        assert!(matches!(
            harness.drive(MatrixInput::ConsumeNextResponse),
            Err(HarnessError::InjectedFailpoint { failpoint: reached }) if reached == failpoint
        ));
        let execution = harness.finish().unwrap();
        let expected_disposition = match failpoint {
            Failpoint::ResponseBeforePrepare => icwm_g0c_harness::ResultDisposition::Failed,
            Failpoint::ResponseAfterCommitBeforeAcknowledge => {
                icwm_g0c_harness::ResultDisposition::Uncertain
            }
            _ => unreachable!("test enumerates the paired response boundaries"),
        };
        assert_eq!(execution.disposition, expected_disposition);
        assert_eq!(execution.failpoints_reached, vec![failpoint]);
        assert_eq!(execution.crashes.len(), 1);
        assert!(execution.response_losses.is_empty());
        let mut artifact: ArtifactReport =
            serde_json::from_str(include_str!("../fixtures/CONTROL-RESULT.json")).unwrap();
        artifact.apply_execution(&execution).unwrap();
        artifact.validate_against_execution(&execution).unwrap();
        artifact.crashes.clear();
        assert!(matches!(
            artifact.validate_against_execution(&execution),
            Err(HarnessError::ArtifactExecutionMismatch)
        ));
    }
}

#[test]
fn stale_writer_failpoint_is_post_effect_and_terminal_errors_accumulate() {
    let effect = EffectIntent::new(
        EffectKind::StaleWriterRejected {
            holder_id: "old".into(),
            lease_epoch: 7,
        },
        StableId::derive("stale-writer", &[b"old", &7_u64.to_be_bytes()]),
    );
    let mut harness = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![effect],
        vec![],
    );
    harness.arm_failpoint(Failpoint::StaleWriterAfterTakeover);
    assert!(matches!(
        harness.drive(MatrixInput::StaleWriterAttempt {
            holder_id: "old".into(),
            lease_epoch: 7,
        }),
        Err(HarnessError::InjectedFailpoint {
            failpoint: Failpoint::StaleWriterAfterTakeover
        })
    ));
    let report = harness.finish().unwrap();
    assert_eq!(report.effects.len(), 1);

    let missing = StableId::derive("missing-operation", &[]);
    let mut multiple = Harness::new(
        ControlAdapter::new(CandidateId::new("control", "1")),
        vec![],
        vec![],
    );
    assert!(multiple.drive(MatrixInput::ConsumeNextResponse).is_err());
    assert!(
        multiple
            .drive(MatrixInput::StepOperation {
                operation_id: missing,
                step: "still-live".into(),
            })
            .is_err()
    );
    let reason = multiple.finish().unwrap().disposition_reason.unwrap();
    assert!(reason.contains("MissingScriptedResponse"));
    assert!(reason.contains("OperationNotActive"));
}

#[test]
fn publication_bindings_are_validated_separately_from_execution() {
    let artifact: ArtifactReport =
        serde_json::from_str(include_str!("../fixtures/CONTROL-RESULT.json")).unwrap();
    let bindings = icwm_g0c_harness::PublicationBindings {
        dependency_graph: artifact.dependency_graph.clone(),
        tested_source_baseline: artifact.harness_commit.clone(),
        component_commits: artifact.component_commits.clone(),
        scenario_hash: artifact.scenario_hash.clone(),
        vector_hashes: artifact.vector_hashes.clone(),
        evidence_hashes: artifact.evidence_hashes.clone(),
        tier: artifact.tier.clone(),
        homeservers: artifact.homeservers.clone(),
        clients: artifact.clients.clone(),
    };
    artifact.validate_publication_bindings(&bindings).unwrap();

    let rejects = |stale: &icwm_g0c_harness::PublicationBindings| {
        assert!(matches!(
            artifact.validate_publication_bindings(stale),
            Err(HarnessError::ArtifactPublicationMismatch)
        ));
    };
    let mut stale = bindings.clone();
    stale.scenario_hash = "0".repeat(64);
    rejects(&stale);

    let mut stale = bindings.clone();
    stale.tier = "live".into();
    rejects(&stale);

    let foreign = icwm_g0c_harness::ArtifactIdentity {
        name: "foreign".into(),
        version: "1".into(),
        stable_id: "0".repeat(64),
    };
    let mut stale = bindings.clone();
    stale.homeservers.push(foreign.clone());
    rejects(&stale);

    let mut stale = bindings.clone();
    stale.clients.push(foreign);
    rejects(&stale);

    let mut stale = bindings.clone();
    stale
        .component_commits
        .insert("unexpected_component".into(), "0".repeat(40));
    rejects(&stale);

    let mut stale = bindings;
    stale
        .component_commits
        .insert("ironclaw_source_baseline".into(), "0".repeat(40));
    rejects(&stale);
}
