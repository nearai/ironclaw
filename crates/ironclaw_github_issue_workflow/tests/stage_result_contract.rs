mod stage_result_contract {
    use ironclaw_github_issue_workflow::{
        GithubIssueStage, StageResultValidationError, validate_stage_result,
    };
    use serde_json::{Value, json};

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
}
