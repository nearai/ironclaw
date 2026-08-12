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
/// Durable identifiers must not be derived from `Debug` (#7144). Both of
/// these cross a persistence boundary — `vector_key` addresses rows in a
/// vector store, the credit fingerprint is written into `submissions.json`
/// and compared on every load to keep an acknowledged notice suppressed —
/// and a `{:?}` derivation meant a variant rename silently re-keyed the
/// first and resurfaced every dismissed notice for the second.
///
/// Frozen at the values `Debug` produced, so nothing already persisted
/// moves. A rename now has to come here.
#[test]
fn durable_identifier_segments_are_frozen_against_variant_renames() {
    assert_eq!(
        [
            CanonicalRepresentationKind::WholeTrace,
            CanonicalRepresentationKind::Turn,
            CanonicalRepresentationKind::ToolSequence,
            CanonicalRepresentationKind::ErrorOutcome,
            CanonicalRepresentationKind::Correction,
        ]
        .map(CanonicalRepresentationKind::vector_key_segment),
        [
            "wholetrace",
            "turn",
            "toolsequence",
            "erroroutcome",
            "correction"
        ],
        "vector keys are durable and cross-service; changing a segment \
         orphans every embedding already indexed under the old key"
    );

    assert_eq!(
        [
            TraceCreditEventKind::Accepted,
            TraceCreditEventKind::RejectedPrivacy,
            TraceCreditEventKind::RejectedDuplicate,
            TraceCreditEventKind::CreditSynced,
            TraceCreditEventKind::Replayable,
            TraceCreditEventKind::NovelCluster,
            TraceCreditEventKind::UnderrepresentedCoverage,
            TraceCreditEventKind::UserCorrectionIncluded,
            TraceCreditEventKind::ConvertedToBenchmark,
            TraceCreditEventKind::CaughtRegression,
            TraceCreditEventKind::UsedForTrainingOrRanking,
            TraceCreditEventKind::ReviewerBonus,
            TraceCreditEventKind::AbusePenalty,
        ]
        .iter()
        .map(TraceCreditEventKind::as_str)
        .collect::<Vec<_>>(),
        vec![
            "Accepted",
            "RejectedPrivacy",
            "RejectedDuplicate",
            "CreditSynced",
            "Replayable",
            "NovelCluster",
            "UnderrepresentedCoverage",
            "UserCorrectionIncluded",
            "ConvertedToBenchmark",
            "CaughtRegression",
            "UsedForTrainingOrRanking",
            "ReviewerBonus",
            "AbusePenalty",
        ],
        "credit-notice fingerprints are persisted in submissions.json with \
         no schema version and no migration; changing a spelling resurfaces \
         every acknowledged notice and duplicates its outbox item"
    );

    // The serde wire tags are the *other* durable form and must also stay
    // PascalCase: `submissions.json` already holds them, so adding
    // `rename_all = "snake_case"` for consistency with the sibling enums
    // would make every existing file fail to deserialize.
    assert_eq!(
        serde_json::to_string(&TraceCreditEventKind::RejectedPrivacy).expect("serializes"),
        "\"RejectedPrivacy\""
    );
}

/// #7144: the trace card stamped `private_corpus_revocable` unconditionally
/// while `retention_policy_for_trace` ranked `allowed_uses` — and they
/// disagree for three of the five consent scopes. The card is what crosses
/// the wire, so the wrong value was the one that shipped.
#[tokio::test]
async fn trace_card_retention_matches_the_ranked_derivation() {
    for (scope, expected) in [
        (
            ConsentScope::DebuggingEvaluation,
            "private_corpus_revocable",
        ),
        (ConsentScope::BenchmarkOnly, "benchmark_revocable"),
        (ConsentScope::ModelTraining, "training_revocable"),
    ] {
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default().set_consent_scopes(vec![scope]),
        );
        let envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");

        assert_eq!(
            envelope.trace_card.retention_policy, expected,
            "the card must carry the policy the allowed uses imply for {scope:?}"
        );
        assert_eq!(
            envelope.trace_card.retention_policy,
            retention_policy_for_trace(&envelope).name,
            "the card and the ranked derivation must not disagree for {scope:?}"
        );
    }
}

/// #7144: `novelty_score` is an unvalidated `Option<f32>` that is re-scored
/// off the on-disk queue, and only its upper bound was enforced — so a
/// negative value from a downstream embedding job passed straight through
/// while its sibling `duplicate_score` was clamped both ways.
#[tokio::test]
async fn novelty_score_is_clamped_at_both_ends() {
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

    envelope.embedding_analysis = Some(EmbeddingAnalysisMetadata {
        embedding_model: None,
        canonical_summary_hash: String::new(),
        trace_vector_id: None,
        nearest_trace_ids: Vec::new(),
        cluster_id: None,
        nearest_cluster_id: None,
        novelty_score: Some(-4.0),
        duplicate_score: None,
        coverage_tags: Vec::new(),
    });
    let low = compute_value_scorecard(&envelope);
    assert!(
        low.novelty >= 0.0,
        "a negative novelty must be clamped, got {}",
        low.novelty
    );

    envelope.embedding_analysis = Some(EmbeddingAnalysisMetadata {
        embedding_model: None,
        canonical_summary_hash: String::new(),
        trace_vector_id: None,
        nearest_trace_ids: Vec::new(),
        cluster_id: None,
        nearest_cluster_id: None,
        novelty_score: Some(99.0),
        duplicate_score: None,
        coverage_tags: Vec::new(),
    });
    let high = compute_value_scorecard(&envelope);
    assert_eq!(high.novelty, 0.85, "the upper cap must still hold");

    // `clamp` bounds both ends but passes NaN straight through
    // (`f32::NAN.clamp(0.0, 0.85)` is NaN), so a non-finite score off the
    // re-scored on-disk queue would poison `raw`, `online_score`, and the
    // persisted `credit_points_estimate`. Non-finite values must be treated
    // as absent: novelty falls back to the event-count heuristic, duplicate
    // to 0.0.
    envelope.embedding_analysis = Some(EmbeddingAnalysisMetadata {
        embedding_model: None,
        canonical_summary_hash: String::new(),
        trace_vector_id: None,
        nearest_trace_ids: Vec::new(),
        cluster_id: None,
        nearest_cluster_id: None,
        novelty_score: Some(f32::NAN),
        duplicate_score: Some(f32::NAN),
        coverage_tags: Vec::new(),
    });
    let poisoned = compute_value_scorecard(&envelope);
    assert!(
        poisoned.novelty.is_finite() && (0.0..=0.85).contains(&poisoned.novelty),
        "a NaN novelty must fall back to the derived default, got {}",
        poisoned.novelty
    );
    assert_eq!(
        poisoned.duplicate_penalty, 0.0,
        "a NaN duplicate score must be treated as absent"
    );
    assert!(
        poisoned.online_score.is_finite() && poisoned.credit_points_estimate.is_finite(),
        "non-finite embedding scores must never poison the credit estimate, got score {} / credit {}",
        poisoned.online_score,
        poisoned.credit_points_estimate
    );
}

/// #7144: dataset eligibility used to be decided by scanning `warnings` for
/// the substring `"quarantined"`, whose sole producer is one English
/// sentence in `privacy_warnings`. Rewording, translating or localising that
/// sentence silently opened the gate — quietly, and in the permissive
/// direction.
///
/// The sabotage the old gate could not survive is the first case here: the
/// prose is replaced wholesale and the gate must still hold.
#[tokio::test]
async fn quarantine_gate_survives_rewording_the_operator_warning() {
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default()
            .set_consent_scopes(vec![ConsentScope::ModelTraining]),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

    // A high-risk trace carries the typed flag from the producer.
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::High;
    envelope.privacy.quarantined = true;
    envelope.privacy.warnings =
        vec!["Sekretartige Inhalte gefunden; bitte zuruckhalten.".to_string()];
    let localised = trace_dataset_eligibility(&envelope, TraceAllowedUse::ModelTraining, false);
    assert!(!localised.eligible);
    assert!(
        localised
            .reasons
            .iter()
            .any(|reason| reason.contains("quarantined")),
        "the gate must key on the typed flag, not the sentence: {:?}",
        localised.reasons
    );

    // An envelope persisted before the flag existed deserializes it as
    // `false`; the typed risk check is what has to close the gate there.
    envelope.privacy.quarantined = false;
    let legacy = trace_dataset_eligibility(&envelope, TraceAllowedUse::ModelTraining, false);
    assert!(
        !legacy.eligible,
        "a pre-flag high-risk envelope must still be held: {:?}",
        legacy.reasons
    );

    // And prose alone must not close it on a low-risk trace: the substring
    // used to be sufficient, so an unrelated warning that happened to
    // contain the word blocked an eligible trace.
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::Low;
    envelope.privacy.warnings = vec!["Downstream job quarantined a sibling trace.".to_string()];
    let prose_only = trace_dataset_eligibility(&envelope, TraceAllowedUse::ModelTraining, false);
    assert!(
        prose_only.eligible,
        "warning prose must not decide eligibility either way: {:?}",
        prose_only.reasons
    );
}

/// The producer and the gate must agree, driven through `redact_trace`
/// rather than by setting the field by hand: an envelope has to arrive from
/// redaction already carrying the typed flag that matches its typed risk,
/// or the gate is reading a value nothing maintains.
#[tokio::test]
async fn redaction_flags_quarantine_on_the_envelope_it_produces() {
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    assert_eq!(
        envelope.privacy.quarantined,
        envelope.privacy.residual_pii_risk == ResidualPiiRisk::High,
        "the typed flag must track the typed risk the producer computed"
    );

    // A server-side re-scrub raises risk monotonically, so it must be able
    // to raise the flag — and must never clear one already set.
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::High;
    envelope.privacy.quarantined = false;
    rescrub_trace_envelope(&mut envelope).expect("re-scrub should succeed");
    assert!(
        envelope.privacy.quarantined,
        "a re-scrub that sees High risk must raise the quarantine flag"
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
