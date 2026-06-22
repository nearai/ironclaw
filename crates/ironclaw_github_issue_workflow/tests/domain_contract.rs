mod domain_contract {
    use ironclaw_github_issue_workflow::{
        GithubIssueCandidateSelector, GithubIssueRef, GithubIssueWorkflowConfig,
        GithubIssueWorkflowError, GithubIssueWorkflowMode, GithubIssueWorkflowRunKey,
        GithubIssueWorkflowRunStatus, GithubIssueWorkflowState, GithubProviderAccountRef,
        WorkflowIdempotencyKey,
    };
    use ironclaw_host_api::{ProjectId, TenantId, UserId};

    #[test]
    fn workflow_run_key_is_stable_for_issue_ref() {
        let issue_ref = GithubIssueRef {
            owner: "nearai".to_string(),
            repo: "ironclaw".to_string(),
            number: 42,
            node_id: Some("node-1".to_string()),
            url: "https://github.com/nearai/ironclaw/issues/42".to_string(),
            default_branch: "main".to_string(),
        };

        let same_issue_with_mutable_fields_changed = GithubIssueRef {
            node_id: Some("node-2".to_string()),
            url: "https://github.example.test/nearai/ironclaw/issues/42".to_string(),
            default_branch: "develop".to_string(),
            ..issue_ref.clone()
        };

        let key = GithubIssueWorkflowRunKey::for_issue(&issue_ref).unwrap();

        assert_eq!(key.as_str(), "github-issue:v1:nearai/ironclaw#42");
        assert_eq!(
            key,
            GithubIssueWorkflowRunKey::for_issue(&same_issue_with_mutable_fields_changed).unwrap()
        );
    }

    #[test]
    fn workflow_state_mode_is_distinct_from_run_status() {
        let state = GithubIssueWorkflowState {
            mode: GithubIssueWorkflowMode::Implementation,
            ..GithubIssueWorkflowState::new(GithubIssueWorkflowMode::Implementation)
        };

        assert_eq!(state.mode, GithubIssueWorkflowMode::Implementation);
        assert_eq!(
            serde_json::to_value(GithubIssueWorkflowRunStatus::Active).unwrap(),
            "active"
        );
        assert_eq!(serde_json::to_value(state.mode).unwrap(), "implementation");
    }

    #[test]
    fn project_scoped_config_rejects_empty_repositories() {
        let config = GithubIssueWorkflowConfig {
            tenant_id: TenantId::new("tenant-1").unwrap(),
            project_id: ProjectId::new("project-1").unwrap(),
            owner_user_id: UserId::new("user-1").unwrap(),
            repositories: Vec::new(),
            candidate_selector: GithubIssueCandidateSelector::default(),
            max_active_runs_per_repo: 1,
            default_run_profile: "default".to_string(),
            provider_account_ref: GithubProviderAccountRef {
                provider: "github".to_string(),
                account_id: "account-1".to_string(),
            },
        };

        let err = config
            .validate()
            .expect_err("empty repositories must reject");

        assert!(matches!(
            err,
            GithubIssueWorkflowError::InvalidConfig { reason }
                if reason.contains("repositories")
        ));
    }

    #[test]
    fn idempotency_key_rejects_empty_and_overlong_values() {
        assert!(WorkflowIdempotencyKey::from_trusted(String::new()).is_err());
        assert!(WorkflowIdempotencyKey::from_trusted("x".repeat(513)).is_err());
        assert!(
            WorkflowIdempotencyKey::from_trusted("issue:nearai/ironclaw#42".to_string()).is_ok()
        );
    }
}
