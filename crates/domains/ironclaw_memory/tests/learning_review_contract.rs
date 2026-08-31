use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_host_api::turn::TurnRunId;
use ironclaw_memory::{
    LearningAction, LearningCandidateStatus, LearningDecision, LearningExplicitness,
    LearningIdempotencyKey, LearningReview, LearningReviewRecord, LearningReviewValidationError,
    LearningScope, MAX_LEARNING_MEMORY_PROPOSALS, MAX_LEARNING_PROPOSAL_BYTES,
    MAX_LEARNING_SKILL_REASON_BYTES, MAX_LEARNING_SOURCE_REFERENCES, MemoryLearningProposal,
    MemoryLearningProposalKind,
};

fn scope() -> LearningScope {
    LearningScope::new(
        TenantId::new("tenant-a").expect("tenant"),
        UserId::new("user-a").expect("user"),
        AgentId::new("agent-a").expect("agent"),
        Some(ProjectId::new("project-a").expect("project")),
    )
}

fn proposal(content: &str) -> MemoryLearningProposal {
    MemoryLearningProposal {
        kind: MemoryLearningProposalKind::Preference,
        content: content.to_string(),
        source_message_indices: vec![1],
        confidence_basis_points: 9_000,
        explicitness: LearningExplicitness::Explicit,
        tainted: false,
    }
}

#[test]
fn learning_review_rejects_too_many_or_oversized_memory_proposals() {
    let too_many = LearningReview {
        memory: vec![proposal("bounded"); MAX_LEARNING_MEMORY_PROPOSALS + 1],
        skill: LearningDecision::skip(),
    };
    assert_eq!(
        too_many.validate(),
        Err(LearningReviewValidationError::TooManyMemoryProposals)
    );

    let oversized = LearningReview {
        memory: vec![proposal(&"x".repeat(MAX_LEARNING_PROPOSAL_BYTES + 1))],
        skill: LearningDecision::skip(),
    };
    assert_eq!(
        oversized.validate(),
        Err(LearningReviewValidationError::MemoryProposalTooLarge)
    );
}

#[test]
fn learning_review_rejects_whitespace_only_memory_proposals() {
    let review = LearningReview {
        memory: vec![proposal(" \t\n")],
        skill: LearningDecision::skip(),
    };

    assert_eq!(
        review.validate(),
        Err(LearningReviewValidationError::EmptyMemoryProposal)
    );
}

#[test]
fn learning_review_rejects_too_many_source_references() {
    let review = LearningReview {
        memory: vec![MemoryLearningProposal {
            source_message_indices: (0..=MAX_LEARNING_SOURCE_REFERENCES as u16).collect(),
            ..proposal("bounded")
        }],
        skill: LearningDecision::skip(),
    };

    assert_eq!(
        review.validate(),
        Err(LearningReviewValidationError::TooManySourceReferences)
    );
}

#[test]
fn learning_review_rejects_duplicate_and_non_monotonic_source_references() {
    for source_message_indices in [vec![1, 1], vec![2, 1]] {
        let review = LearningReview {
            memory: vec![MemoryLearningProposal {
                source_message_indices,
                ..proposal("bounded")
            }],
            skill: LearningDecision::skip(),
        };

        assert_eq!(
            review.validate(),
            Err(LearningReviewValidationError::InvalidSourceReferences)
        );
    }
}

#[test]
fn learning_review_rejects_skip_reasons_and_sources() {
    let with_reason = LearningReview {
        memory: Vec::new(),
        skill: LearningDecision {
            reason: Some("not reusable".to_string()),
            ..LearningDecision::skip()
        },
    };
    assert_eq!(
        with_reason.validate(),
        Err(LearningReviewValidationError::InvalidSkillDecision)
    );

    let with_source = LearningReview {
        memory: Vec::new(),
        skill: LearningDecision {
            source_message_indices: vec![1],
            ..LearningDecision::skip()
        },
    };
    assert_eq!(
        with_source.validate(),
        Err(LearningReviewValidationError::InvalidSkillDecision)
    );
}

#[test]
fn learning_review_rejects_missing_and_blank_distill_reasons() {
    for reason in [None, Some(" \t\n".to_string())] {
        let review = LearningReview {
            memory: Vec::new(),
            skill: LearningDecision {
                action: LearningAction::Distill,
                reason,
                source_message_indices: vec![1],
                tainted: false,
            },
        };

        assert_eq!(
            review.validate(),
            Err(LearningReviewValidationError::InvalidSkillDecision)
        );
    }
}

#[test]
fn learning_review_rejects_oversized_skill_reasons() {
    let review = LearningReview {
        memory: Vec::new(),
        skill: LearningDecision {
            action: LearningAction::Distill,
            reason: Some("x".repeat(MAX_LEARNING_SKILL_REASON_BYTES + 1)),
            source_message_indices: vec![1],
            tainted: false,
        },
    };

    assert_eq!(
        review.validate(),
        Err(LearningReviewValidationError::SkillReasonTooLarge)
    );
}

#[test]
fn learning_review_rejects_unknown_output_fields() {
    let json = r#"{
        "memory": [],
        "skill": {"action": "skip", "reason": null, "source_message_indices": []},
        "unexpected": true
    }"#;
    assert!(serde_json::from_str::<LearningReview>(json).is_err());
}

#[test]
fn learning_idempotency_key_rejects_malformed_values() {
    let malformed = r#""learning-review:not-a-run""#;
    assert!(serde_json::from_str::<LearningIdempotencyKey>(malformed).is_err());
    let valid = format!(r#""learning-review:{}""#, TurnRunId::new());
    assert!(serde_json::from_str::<LearningIdempotencyKey>(&valid).is_ok());
}

#[test]
fn learning_review_rejects_invalid_confidence_and_source_references() {
    let invalid_confidence = LearningReview {
        memory: vec![MemoryLearningProposal {
            confidence_basis_points: 10_001,
            ..proposal("bounded")
        }],
        skill: LearningDecision::skip(),
    };
    assert!(invalid_confidence.validate().is_err());

    let missing_source = LearningReview {
        memory: vec![MemoryLearningProposal {
            source_message_indices: Vec::new(),
            ..proposal("bounded")
        }],
        skill: LearningDecision::skip(),
    };
    assert!(missing_source.validate().is_err());
}

#[test]
fn record_seals_scope_run_and_candidate_status() {
    let run_id = TurnRunId::new();
    let review = LearningReview {
        memory: vec![proposal("Use concise status reports")],
        skill: LearningDecision {
            action: LearningAction::Distill,
            reason: Some("The run contains a reusable procedure".to_string()),
            source_message_indices: vec![1],
            tainted: false,
        },
    };

    let record = LearningReviewRecord::new(run_id, scope(), review).expect("valid record");
    assert_eq!(record.run_id, run_id);
    assert_eq!(record.status, LearningCandidateStatus::Candidate);
    assert_eq!(
        record.idempotency_key.as_str(),
        format!("learning-review:{run_id}")
    );
    assert_eq!(record.scope.user_id().as_str(), "user-a");
}
