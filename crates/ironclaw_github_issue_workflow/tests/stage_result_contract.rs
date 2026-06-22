mod stage_result_contract {
    use ironclaw_github_issue_workflow::{
        AcceptedStageResult, GithubIssueStage, GithubIssueStageRunId, GithubIssueWorkflowRunId,
        StageResultAttempt, StageResultBinding, StageResultReportDecision, StageResultReportError,
        StageResultValidationError, evaluate_stage_result_attempt, validate_stage_result,
    };
    use ironclaw_turns::TurnRunId;
    use serde_json::{Value, json};

    fn workflow_run_id(value: &str) -> GithubIssueWorkflowRunId {
        GithubIssueWorkflowRunId::from_trusted(value.to_string()).unwrap()
    }

    fn stage_run_id(value: &str) -> GithubIssueStageRunId {
        GithubIssueStageRunId::from_trusted(value.to_string()).unwrap()
    }

    fn result(payload: Value) -> Value {
        json!({
            "outcome": "completed",
            "summary": "stage completed",
            "evidence": [],
            "next_actions": [],
            "payload": payload,
        })
    }

    fn implementation_payload() -> Value {
        json!({
            "changed_files": ["src/lib.rs"],
            "commands_run": ["cargo test -p ironclaw_github_issue_workflow stage_result_contract"],
            "test_evidence": ["stage_result_contract passed"],
            "pr_ready": true,
        })
    }

    fn valid_payload_for(stage: GithubIssueStage) -> (&'static str, Value) {
        match stage {
            GithubIssueStage::Triage => (
                "triage.v1",
                json!({
                    "is_reproducible": true,
                    "suspected_area": "github_issue_workflow",
                    "risk": "medium",
                    "recommended_next_stage": "planning",
                }),
            ),
            GithubIssueStage::Planning => (
                "planning.v1",
                json!({
                    "plan_items": ["add strict payload validation"],
                    "files_to_inspect_or_change": ["crates/ironclaw_github_issue_workflow/src/stage_schemas.rs"],
                    "test_strategy": "stage result contract regression",
                    "confidence": 0.91,
                }),
            ),
            GithubIssueStage::Implementation => ("implementation.v1", implementation_payload()),
            GithubIssueStage::PrSynthesis => (
                "pr_synthesis.v1",
                json!({
                    "title": "Fix strict stage validation",
                    "body": "Reject unknown payload fields.",
                    "branch_name": "codex/github-stage-validation",
                    "base_branch": "main",
                    "head_sha": "abc123",
                }),
            ),
            GithubIssueStage::CiRepair => (
                "ci_repair.v1",
                json!({
                    "failing_checks": ["clippy"],
                    "diagnosis": "unknown payload fields were accepted",
                    "changed_files": ["crates/ironclaw_github_issue_workflow/src/stage_schemas.rs"],
                    "commands_run": ["cargo test -p ironclaw_github_issue_workflow stage_result_contract"],
                }),
            ),
            GithubIssueStage::ReviewResponse => (
                "review_response.v1",
                json!({
                    "addressed_comments": ["reject unknown stage payload keys"],
                    "remaining_comments": [],
                    "commands_run": ["cargo test -p ironclaw_github_issue_workflow stage_result_contract"],
                }),
            ),
        }
    }

    fn binding(stage: GithubIssueStage) -> StageResultBinding {
        StageResultBinding {
            workflow_run_id: workflow_run_id("workflow-run-1"),
            stage_run_id: stage_run_id("stage-run-1"),
            turn_run_id: TurnRunId::new(),
            stage,
            schema_version: "implementation.v1".to_string(),
            completion_nonce: "nonce-1".to_string(),
        }
    }

    fn attempt(binding: &StageResultBinding, result: Value) -> StageResultAttempt {
        StageResultAttempt {
            workflow_run_id: binding.workflow_run_id.clone(),
            stage_run_id: binding.stage_run_id.clone(),
            turn_run_id: binding.turn_run_id,
            stage: binding.stage.clone(),
            schema_version: binding.schema_version.clone(),
            completion_nonce: binding.completion_nonce.clone(),
            result,
        }
    }

    fn accepted_from(decision: StageResultReportDecision) -> AcceptedStageResult {
        let StageResultReportDecision::Accepted { accepted_result } = decision else {
            panic!("expected accepted stage result decision");
        };
        accepted_result
    }

    #[test]
    fn implementation_result_requires_changed_files_and_test_evidence() {
        let missing_changed_files = validate_stage_result(
            GithubIssueStage::Implementation,
            "implementation.v1",
            result(json!({
                "commands_run": [],
                "test_evidence": [],
                "pr_ready": false,
            })),
        )
        .unwrap_err();
        let missing_test_evidence = validate_stage_result(
            GithubIssueStage::Implementation,
            "implementation.v1",
            result(json!({
                "changed_files": [],
                "commands_run": [],
                "pr_ready": false,
            })),
        )
        .unwrap_err();

        assert!(matches!(
            missing_changed_files,
            StageResultValidationError::MissingPayloadField {
                field: "changed_files",
                ..
            }
        ));
        assert!(matches!(
            missing_test_evidence,
            StageResultValidationError::MissingPayloadField {
                field: "test_evidence",
                ..
            }
        ));
    }

    #[test]
    fn pr_synthesis_result_requires_branch_and_head_sha() {
        let missing_branch_name = validate_stage_result(
            GithubIssueStage::PrSynthesis,
            "pr_synthesis.v1",
            result(json!({
                "title": "Fix workflow result validation",
                "body": "Implements strict validation.",
                "base_branch": "main",
                "head_sha": "abc123",
            })),
        )
        .unwrap_err();
        let missing_head_sha = validate_stage_result(
            GithubIssueStage::PrSynthesis,
            "pr_synthesis.v1",
            result(json!({
                "title": "Fix workflow result validation",
                "body": "Implements strict validation.",
                "branch_name": "codex/workflow-stage-results",
                "base_branch": "main",
            })),
        )
        .unwrap_err();

        assert!(matches!(
            missing_branch_name,
            StageResultValidationError::MissingPayloadField {
                field: "branch_name",
                ..
            }
        ));
        assert!(matches!(
            missing_head_sha,
            StageResultValidationError::MissingPayloadField {
                field: "head_sha",
                ..
            }
        ));
    }

    #[test]
    fn supported_stage_payloads_reject_unknown_fields() {
        for stage in [
            GithubIssueStage::Triage,
            GithubIssueStage::Planning,
            GithubIssueStage::Implementation,
            GithubIssueStage::PrSynthesis,
            GithubIssueStage::CiRepair,
            GithubIssueStage::ReviewResponse,
        ] {
            let (schema_version, mut payload) = valid_payload_for(stage.clone());
            payload
                .as_object_mut()
                .unwrap()
                .insert("unexpected_payload_key".to_string(), json!("surprise"));

            let error =
                validate_stage_result(stage.clone(), schema_version, result(payload)).unwrap_err();

            assert!(matches!(
                error,
                StageResultValidationError::UnknownPayloadField {
                    field,
                    schema_version: rejected_schema_version,
                    ..
                } if field == "unexpected_payload_key"
                    && rejected_schema_version == schema_version
            ));
        }
    }

    #[test]
    fn first_valid_stage_result_wins() {
        let binding = binding(GithubIssueStage::Implementation);
        let first = attempt(&binding, result(implementation_payload()));
        let first_decision = evaluate_stage_result_attempt(&binding, None, first).unwrap();
        let accepted = accepted_from(first_decision);

        let second = attempt(
            &binding,
            result(json!({
                "changed_files": ["src/other.rs"],
                "commands_run": ["cargo test -p ironclaw_github_issue_workflow stage_result_contract"],
                "test_evidence": ["different evidence"],
                "pr_ready": false,
            })),
        );
        let second_decision =
            evaluate_stage_result_attempt(&binding, Some(&accepted), second).unwrap_err();

        assert!(matches!(
            second_decision,
            StageResultReportError::ConflictingAcceptedResult
        ));
    }

    #[test]
    fn duplicate_identical_stage_result_replays_ack() {
        let binding = binding(GithubIssueStage::Implementation);
        let first = attempt(&binding, result(implementation_payload()));
        let first_decision = evaluate_stage_result_attempt(&binding, None, first.clone()).unwrap();
        let accepted = accepted_from(first_decision);

        let duplicate = evaluate_stage_result_attempt(&binding, Some(&accepted), first).unwrap();

        assert!(matches!(
            duplicate,
            StageResultReportDecision::Duplicate {
                accepted_result
            } if accepted_result == accepted
        ));
    }

    #[test]
    fn mismatched_completion_nonce_is_rejected() {
        let binding = binding(GithubIssueStage::Implementation);
        let mut bad_attempt = attempt(&binding, result(implementation_payload()));
        bad_attempt.completion_nonce = "wrong-nonce".to_string();

        let decision = evaluate_stage_result_attempt(&binding, None, bad_attempt).unwrap_err();

        assert!(matches!(
            decision,
            StageResultReportError::MismatchedBinding {
                field: "completion_nonce"
            }
        ));
    }

    #[test]
    fn stale_stage_attempt_is_rejected() {
        let binding = binding(GithubIssueStage::Implementation);
        let mut stale_attempt = attempt(&binding, result(implementation_payload()));
        stale_attempt.stage_run_id = stage_run_id("stage-run-old");

        let decision = evaluate_stage_result_attempt(&binding, None, stale_attempt).unwrap_err();

        assert!(matches!(decision, StageResultReportError::StaleAttempt));
    }

    #[test]
    fn invalid_result_records_validation_failure() {
        let binding = binding(GithubIssueStage::Implementation);
        let invalid_attempt = attempt(
            &binding,
            result(json!({
                "commands_run": [],
                "test_evidence": [],
                "pr_ready": false,
            })),
        );

        let decision = evaluate_stage_result_attempt(&binding, None, invalid_attempt).unwrap();

        assert!(matches!(
            decision,
            StageResultReportDecision::ValidationFailed {
                error: StageResultValidationError::MissingPayloadField {
                    field: "changed_files",
                    ..
                }
            }
        ));
    }
}
