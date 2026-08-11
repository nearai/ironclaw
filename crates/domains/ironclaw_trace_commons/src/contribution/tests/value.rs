//! Value scoring, the research scorecard, and process-evaluator labels.

use crate::contribution::*;

use super::support::*;

#[tokio::test]
async fn value_score_caps_novelty_and_records_scorecard() {
    let mut raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    raw.embedding_analysis = Some(EmbeddingAnalysisMetadata {
        embedding_model: Some("test-embedding".to_string()),
        canonical_summary_hash: "sha256:test".to_string(),
        trace_vector_id: Some("vector-1".to_string()),
        nearest_trace_ids: Vec::new(),
        cluster_id: Some("cluster-1".to_string()),
        nearest_cluster_id: Some("cluster-1".to_string()),
        novelty_score: Some(99.0),
        duplicate_score: Some(0.0),
        coverage_tags: Vec::new(),
    });
    let envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

    let estimate = estimate_initial_credit(&envelope);
    assert_eq!(estimate.scorecard.novelty, 0.85);
    assert_eq!(
        estimate.credit_points_pending,
        estimate.scorecard.credit_points_estimate
    );
}
#[tokio::test]
async fn research_scorecard_extension_fields_default_for_older_envelopes() {
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    let mut json = serde_json::to_value(&envelope).expect("envelope serializes");
    let object = json.as_object_mut().expect("envelope is a json object");
    object.remove("hindsight");
    object.remove("training_dynamics");
    object.remove("process_evaluation");

    let decoded: TraceContributionEnvelope =
        serde_json::from_value(json).expect("older envelope deserializes");

    assert_eq!(decoded.hindsight, None);
    assert_eq!(decoded.training_dynamics, None);
    assert_eq!(decoded.process_evaluation, None);
}
#[tokio::test]
async fn process_evaluator_labels_allow_partial_future_payloads() {
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    let mut json = serde_json::to_value(&envelope).expect("envelope serializes");
    let object = json.as_object_mut().expect("envelope is a json object");
    object.insert(
        "process_evaluation".to_string(),
        serde_json::json!({
            "overall_score": 0.66,
            "labels": ["proper_verification"]
        }),
    );

    let decoded: TraceContributionEnvelope =
        serde_json::from_value(json).expect("partial process evaluation deserializes");

    let process_evaluation = decoded
        .process_evaluation
        .expect("process evaluation should be preserved");
    assert_eq!(process_evaluation.evaluator_version, "");
    assert_eq!(process_evaluation.overall_score, Some(0.66));
    assert_eq!(
        process_evaluation.labels,
        vec![ProcessEvaluatorLabel::ProperVerification]
    );
}
#[tokio::test]
async fn process_evaluator_labels_do_not_require_raw_content() {
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    envelope.process_evaluation = Some(
        ProcessEvaluationLabels::default()
            .set_evaluator_version("process-evaluator-v1")
            .set_labels(vec![
                ProcessEvaluatorLabel::CorrectToolSelection,
                ProcessEvaluatorLabel::MissingVerification,
            ])
            .set_tool_selection(ProcessEvalRating::Pass)
            .set_tool_argument_quality(ProcessEvalRating::Unknown)
            .set_tool_ordering(ProcessEvalRating::Partial)
            .set_verification(ProcessEvalRating::Fail)
            .set_side_effect_safety(ProcessEvalRating::Pass)
            .set_overall_score(0.72),
    );
    envelope.hindsight = Some(
        HindsightRelabelingCandidate::default()
            .set_achieved_subgoals(vec!["redacted_subgoal:diagnosed_tool_failure".to_string()])
            .set_failure_type(TraceFailureMode::MissingVerification)
            .set_recoverability_score(0.8)
            .set_benchmark_candidate(true)
            .set_relabeled_training_candidate(true),
    );
    envelope.training_dynamics = Some(TrainingDynamicsSignals {
        mean_confidence: Some(0.61),
        variability: Some(0.29),
        correctness: Some(0.5),
        cartography_bucket: Some(CartographyBucket::Ambiguous),
    });

    let json = serde_json::to_string(&envelope).expect("envelope serializes");
    let decoded: TraceContributionEnvelope =
        serde_json::from_str(&json).expect("envelope deserializes");

    assert!(json.contains("process_evaluation"));
    assert!(json.contains("training_dynamics"));
    assert!(json.contains("hindsight"));
    assert!(!json.contains("raw_content"));
    assert!(!json.contains("raw_tool"));
    assert!(!json.contains("hidden_reasoning"));
    assert_eq!(
        decoded
            .process_evaluation
            .as_ref()
            .expect("process labels present")
            .labels,
        vec![
            ProcessEvaluatorLabel::CorrectToolSelection,
            ProcessEvaluatorLabel::MissingVerification,
        ]
    );
}
