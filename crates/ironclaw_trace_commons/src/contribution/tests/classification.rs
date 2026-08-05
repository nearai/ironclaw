//! Canonical representations, dataset eligibility and hold classes, capture, and scoped on-disk layout.

use std::path::PathBuf;

use chrono::Utc;
use uuid::Uuid;

use crate::contribution::*;

use super::support::*;

#[tokio::test]
async fn canonical_summary_uses_redacted_content_only() {
    let options = RecordedTraceContributionOptions::default()
        .set_include_message_text(true)
        .set_include_tool_payloads(true);
    let raw = RawTraceContribution::from_recorded_trace(&sample_trace(), options);
    let envelope = DeterministicTraceRedactor::with_known_path_prefixes([PathBuf::from(
        "/Users/alice/project",
    )])
    .redact_trace(raw)
    .await
    .expect("redaction should succeed");

    let summary = canonical_summary_for_embedding(&envelope);
    assert!(summary.contains("<PRIVATE_LOCAL_PATH_"));
    assert!(!summary.contains("/Users/alice/project"));
    assert!(!summary.contains("abcdefghijklmnopqrstuvwxyz"));
}
#[tokio::test]
async fn canonical_representations_use_only_redacted_private_values() {
    let mut raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default()
            .set_include_message_text(true)
            .set_include_tool_payloads(true)
            .set_consent_scopes(vec![ConsentScope::ModelTraining]),
    );
    raw.outcome = OutcomeMetadata::default()
        .set_user_feedback(UserFeedback::Correction)
        .set_task_success(TaskSuccess::Partial)
        .set_failure_modes(vec![TraceFailureMode::UserIntentMisread])
        .set_human_correction(
            "Use alice@example.com and /Users/alice/project/fix.md as the correction",
        );
    let envelope = DeterministicTraceRedactor::with_known_path_prefixes([PathBuf::from(
        "/Users/alice/project",
    )])
    .redact_trace(raw)
    .await
    .expect("redaction should succeed");

    let representations = canonical_representations_for_embedding(&envelope);
    let joined = representations
        .iter()
        .map(|representation| representation.content.as_str())
        .collect::<Vec<_>>()
        .join("\n---\n");

    assert!(
        representations
            .iter()
            .any(|representation| representation.kind == CanonicalRepresentationKind::WholeTrace)
    );
    assert!(
        representations
            .iter()
            .any(|representation| representation.kind == CanonicalRepresentationKind::Turn)
    );
    assert!(
        representations
            .iter()
            .any(|representation| representation.kind == CanonicalRepresentationKind::ToolSequence)
    );
    assert!(
        representations
            .iter()
            .any(|representation| representation.kind == CanonicalRepresentationKind::ErrorOutcome)
    );
    assert!(
        representations
            .iter()
            .any(|representation| representation.kind == CanonicalRepresentationKind::Correction)
    );
    assert!(joined.contains("<PRIVATE_EMAIL_"));
    assert!(joined.contains("<PRIVATE_LOCAL_PATH_"));
    assert!(!joined.contains("alice@example.com"));
    assert!(!joined.contains("/Users/alice/project"));
    assert!(!joined.contains("abcdefghijklmnopqrstuvwxyz"));
    assert!(
        representations
            .iter()
            .all(|representation| representation.canonical_hash.starts_with("sha256:"))
    );
    assert!(
        representations
            .iter()
            .all(|representation| representation.vector_key.starts_with("trace:"))
    );
}
#[tokio::test]
async fn dataset_eligibility_gates_consent_revocation_and_privacy_risk() {
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default()
            .set_consent_scopes(vec![ConsentScope::ModelTraining]),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

    let eligible = trace_dataset_eligibility(&envelope, TraceAllowedUse::ModelTraining, false);
    assert!(eligible.eligible);
    assert_eq!(
        eligible.retention_policy.class,
        TraceRetentionClass::TrainingRevocable
    );

    let revoked = trace_dataset_eligibility(&envelope, TraceAllowedUse::ModelTraining, true);
    assert!(!revoked.eligible);
    assert!(
        revoked
            .reasons
            .iter()
            .any(|reason| reason.contains("revoked"))
    );

    let outside_scope =
        trace_dataset_eligibility(&envelope, TraceAllowedUse::BenchmarkGeneration, false);
    assert!(!outside_scope.eligible);
    assert!(
        outside_scope
            .reasons
            .iter()
            .any(|reason| reason.contains("outside consent"))
    );

    envelope.privacy.residual_pii_risk = ResidualPiiRisk::Medium;
    let medium_training =
        trace_dataset_eligibility(&envelope, TraceAllowedUse::ModelTraining, false);
    assert!(!medium_training.eligible);
    assert!(
        medium_training
            .reasons
            .iter()
            .any(|reason| reason.contains("medium residual privacy risk"))
    );

    envelope.privacy.residual_pii_risk = ResidualPiiRisk::High;
    let high_eval = trace_dataset_eligibility(&envelope, TraceAllowedUse::Evaluation, false);
    assert!(!high_eval.eligible);
    assert!(
        high_eval
            .reasons
            .iter()
            .any(|reason| reason.contains("high residual privacy risk"))
    );
}
#[tokio::test]
async fn medium_pii_tool_trace_auto_submits_while_high_is_held() {
    // Below-High residual PII risk must auto-submit: the manual-approval
    // eligibility gate fires only on High, and the value scorecard no
    // longer crushes a Medium tool trace below the 0.35 submission gate.
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

    let policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_require_manual_approval_when_pii_detected(true);
    assert_eq!(policy.min_submission_score, 0.35, "default gate is 0.35");

    // Medium: clears the score gate and auto-submits (no manual review).
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::Medium;
    apply_credit_estimate_to_envelope(&mut envelope);
    assert!(
        envelope.value.submission_score >= policy.min_submission_score,
        "medium-risk tool trace must clear the score gate, got {}",
        envelope.value.submission_score
    );
    assert!(
        matches!(
            trace_autonomous_eligibility(&envelope, &policy),
            TraceQueueEligibility::Submit
        ),
        "medium-risk tool trace must auto-submit, not hold for manual review"
    );

    // High: still held (and its score collapses to zero via the gate).
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::High;
    apply_credit_estimate_to_envelope(&mut envelope);
    assert!(
        matches!(
            trace_autonomous_eligibility(&envelope, &policy),
            TraceQueueEligibility::Hold { .. }
        ),
        "high-risk trace must remain held"
    );
}
#[tokio::test]
async fn empty_allowed_uses_envelope_fails_closed_not_submitted() {
    // A public_attribution-only consent scope grants no trace-content
    // allowed-uses; such an envelope must never be submitted, even with an
    // otherwise-permissive auto-submit policy or an explicit manual-review
    // authorization (there is nothing to submit it for).
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    envelope.trace_card.allowed_uses = Vec::new();

    let permissive = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_auto_submit_high_value_traces(true)
        .set_min_submission_score(0.0);
    assert!(
        matches!(
            trace_autonomous_eligibility(&envelope, &permissive),
            TraceQueueEligibility::Hold {
                kind: TraceQueueHoldKind::PolicyGate,
                ..
            }
        ),
        "empty allowed-uses must fail closed under a permissive auto-submit policy"
    );

    // Even an explicit manual-review authorization cannot submit it.
    envelope.manual_review_authorized = true;
    assert!(
        matches!(
            trace_autonomous_eligibility(&envelope, &permissive),
            TraceQueueEligibility::Hold { .. }
        ),
        "empty allowed-uses must fail closed even when manual_review_authorized"
    );
}
#[tokio::test]
async fn eligibility_hold_kind_separates_manual_review_from_policy_gate() {
    // The hold kind must distinguish a PII manual-review hold (which is
    // retained for the user to authorize) from a policy/value gate (which
    // is not review-worthy), so the held-review surface is not polluted
    // with low-value traces.
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    apply_credit_estimate_to_envelope(&mut envelope);

    // High residual PII risk + manual-approval policy => ManualReview.
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::High;
    let manual_policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_require_manual_approval_when_pii_detected(true);
    assert!(matches!(
        trace_autonomous_eligibility(&envelope, &manual_policy),
        TraceQueueEligibility::Hold {
            kind: TraceQueueHoldKind::ManualReview,
            ..
        }
    ));

    // Below-threshold score (no PII concern) => PolicyGate, not review.
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::Low;
    let strict_policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_min_submission_score(1.0);
    assert!(matches!(
        trace_autonomous_eligibility(&envelope, &strict_policy),
        TraceQueueEligibility::Hold {
            kind: TraceQueueHoldKind::PolicyGate,
            ..
        }
    ));
}
#[tokio::test]
async fn derived_artifact_invalidation_marker_uses_hashes_not_raw_handles() {
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    let marker = derived_artifact_invalidation_marker(&envelope, "user revoked consent");
    let json = serde_json::to_string(&marker).expect("marker serializes");

    assert_eq!(marker.submission_id, envelope.submission_id);
    assert!(marker.revocation_handle_hash.starts_with("sha256:"));
    assert!(!json.contains(&envelope.contributor.revocation_handle.to_string()));
    assert!(
        marker
            .artifact_prefixes
            .contains(&format!("embedding:{}", envelope.trace_id))
    );
}
#[test]
fn capture_turns_reconstructs_tool_calls_from_conversation_messages() {
    let now = Utc::now();
    let messages = vec![
        crate::ConversationMessage {
            id: Uuid::new_v4(),
            role: "user".to_string(),
            content: "Please inspect the build".to_string(),
            created_at: now,
        },
        crate::ConversationMessage {
            id: Uuid::new_v4(),
            role: "tool_calls".to_string(),
            content: serde_json::json!({
                "calls": [{
                    "name": "shell",
                    "result_preview": "build succeeded",
                    "rationale": "run the project check"
                }]
            })
            .to_string(),
            created_at: now,
        },
        crate::ConversationMessage {
            id: Uuid::new_v4(),
            role: "assistant".to_string(),
            content: "The build is clean.".to_string(),
            created_at: now,
        },
    ];

    let turns = capture_turns_from_conversation_messages(&messages);

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].user_input, "Please inspect the build");
    assert_eq!(turns[0].response.as_deref(), Some("The build is clean."));
    assert_eq!(turns[0].tool_calls.len(), 1);
    assert_eq!(turns[0].tool_calls[0].name, "shell");
    assert_eq!(
        turns[0].tool_calls[0].result_preview.as_deref(),
        Some("build succeeded")
    );
}
#[test]
fn scoped_trace_state_uses_hashed_isolated_paths_and_refs() {
    let alice = trace_contribution_dir_for_scope(Some("tenant-a:user-alice"));
    let bob = trace_contribution_dir_for_scope(Some("tenant-b:user-bob"));
    let alice_path = alice.to_string_lossy();

    assert_ne!(alice, bob);
    assert!(!alice_path.contains("tenant-a"));
    assert!(!alice_path.contains("user-alice"));
    assert_eq!(
        local_pseudonymous_contributor_id("tenant-a:user-alice"),
        local_pseudonymous_contributor_id("tenant-a:user-alice")
    );
    assert_ne!(
        local_pseudonymous_contributor_id("tenant-a:user-alice"),
        local_pseudonymous_contributor_id("tenant-b:user-bob")
    );
    assert!(local_pseudonymous_tenant_scope_ref("tenant-a").starts_with("tenant_sha256:"));
}
