mod prompt_snapshot_contract {
    use ironclaw_github_issue_workflow::{
        EngineeredWorkflowSnapshot, GithubIssueSnapshot, GithubIssueStage, ProviderContentSummary,
        RepositorySnapshot, StageConstraintSnapshot, StageResultSummary, WorkflowStateSnapshot,
        WorkflowWorkspaceSnapshot, render_stage_prompt, snapshot_hash, stage_result_schema_version,
    };
    use sha2::{Digest, Sha256};
    use std::{fs, path::PathBuf};

    fn implementation_snapshot() -> EngineeredWorkflowSnapshot {
        EngineeredWorkflowSnapshot {
            issue: GithubIssueSnapshot {
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                number: 42,
                title: "workflow bug fails on retry".to_string(),
                url: "https://github.com/nearai/ironclaw/issues/42".to_string(),
                default_branch: "main".to_string(),
                state: "open".to_string(),
                labels: vec!["bug".to_string(), "workflow".to_string()],
                summary: "Retrying a busy workflow stage can leave the run stuck.".to_string(),
                provider_content_summaries: vec![ProviderContentSummary {
                    source_ref: "issue-comment:100".to_string(),
                    author: Some("reporter".to_string()),
                    summary: "Reporter confirmed the retry happened after a lease timeout."
                        .to_string(),
                    trust: "untrusted_provider_content".to_string(),
                }],
            },
            workflow: WorkflowStateSnapshot {
                workflow_run_id: "workflow-run-42".to_string(),
                workflow_policy_key: "github_issue_bugfix".to_string(),
                workflow_policy_version: "2026-06-22".to_string(),
                status: "active".to_string(),
                mode: "implementation".to_string(),
                active_stage_run_id: Some("stage-run-implementation".to_string()),
                event_cursor: 7,
                workflow_run_version: 3,
                active_block_summary: None,
                plan: vec![
                    "reproduce stuck retry".to_string(),
                    "patch workflow retry handling".to_string(),
                    "run focused workflow tests".to_string(),
                ],
            },
            repository: RepositorySnapshot {
                owner: "nearai".to_string(),
                name: "ironclaw".to_string(),
                default_branch: "main".to_string(),
                base_ref: Some("refs/heads/main".to_string()),
                base_sha: Some("abc123".to_string()),
                working_branch: Some("codex/github-bug-workflow".to_string()),
                head_sha: Some("def456".to_string()),
                primary_pr_url: None,
            },
            previous_stage_results: vec![StageResultSummary {
                stage: GithubIssueStage::Planning,
                outcome: "completed".to_string(),
                summary: "Plan narrowed the bug to workflow retry state.".to_string(),
                evidence: vec!["policy_contract reproduces retry path".to_string()],
            }],
            workspace: Some(WorkflowWorkspaceSnapshot {
                workspace_session_id: Some("workspace-session-42".to_string()),
                thread_id: Some("thread-42".to_string()),
                turn_run_id: Some("turn-42".to_string()),
                mount_alias: Some("repo".to_string()),
                virtual_root: "/workspace/repo".to_string(),
                changed_files: vec!["crates/ironclaw_github_issue_workflow/src/policy.rs"
                    .to_string()],
            }),
            constraints: StageConstraintSnapshot {
                stage: GithubIssueStage::Implementation,
                stage_goal: "Patch the workflow and collect verification evidence.".to_string(),
                allowed_capabilities: vec![
                    "filesystem.read".to_string(),
                    "filesystem.write".to_string(),
                    "shell.test".to_string(),
                    "github.read".to_string(),
                    "builtin.workflow_report_stage_result".to_string(),
                ],
                disallowed_capabilities: vec![
                    "github.create_pull_request".to_string(),
                    "github.create_issue_comment".to_string(),
                ],
                result_schema_version: "implementation.v1".to_string(),
                completion_tool: "builtin.workflow_report_stage_result".to_string(),
                provider_write_policy: "Return provider-write intent only; workflow provider actions perform GitHub writes."
                    .to_string(),
            },
        }
    }

    fn planning_snapshot() -> EngineeredWorkflowSnapshot {
        let mut snapshot = implementation_snapshot();
        snapshot.workflow.mode = "planning".to_string();
        snapshot.workflow.active_stage_run_id = Some("stage-run-planning".to_string());
        snapshot.workspace = None;
        snapshot.constraints = StageConstraintSnapshot {
            stage: GithubIssueStage::Planning,
            stage_goal: "Produce an implementation plan and test strategy.".to_string(),
            allowed_capabilities: vec![
                "filesystem.read".to_string(),
                "github.read".to_string(),
                "builtin.workflow_report_stage_result".to_string(),
            ],
            disallowed_capabilities: vec![
                "filesystem.write".to_string(),
                "github.create_pull_request".to_string(),
                "github.create_issue_comment".to_string(),
            ],
            result_schema_version: "planning.v1".to_string(),
            completion_tool: "builtin.workflow_report_stage_result".to_string(),
            provider_write_policy: "Return provider-write intent only; workflow provider actions perform GitHub writes."
                .to_string(),
        };
        snapshot
    }

    fn snapshot_for_stage(stage: GithubIssueStage) -> EngineeredWorkflowSnapshot {
        let mut snapshot = implementation_snapshot();
        snapshot.workflow.mode = format!("{stage:?}").to_lowercase();
        snapshot.workflow.active_stage_run_id = Some(format!("stage-run-{stage:?}").to_lowercase());
        snapshot.constraints.stage = stage.clone();
        snapshot.constraints.result_schema_version =
            stage_result_schema_version(&stage).to_string();
        snapshot
    }

    fn prompt_pack_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("prompts/github_issue_bugfix/v1")
    }

    fn sha256_hex(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    #[test]
    fn snapshot_hash_is_stable_for_same_snapshot() {
        let snapshot = implementation_snapshot();

        let first = snapshot_hash(&snapshot).unwrap();
        let second = snapshot_hash(&snapshot).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn snapshot_excludes_raw_host_paths() {
        let snapshot = implementation_snapshot();
        let serialized = serde_json::to_string(&snapshot).unwrap();

        assert!(!serialized.contains("/Users/ben/near/ironclaw"));
        assert!(!serialized.contains("host_path"));
        assert!(!serialized.contains("backend_error"));
        assert!(!serialized.contains("\"body\""));
        assert!(!serialized.contains("\"comments\""));
        assert!(!serialized.contains("GITHUB_TOKEN"));
    }

    #[test]
    fn implementation_prompt_names_result_tool_and_schema() {
        let snapshot = implementation_snapshot();
        let bundle = render_stage_prompt(GithubIssueStage::Implementation, &snapshot).unwrap();

        assert_eq!(bundle.prompt_ref, "github_issue_bugfix/v1/implement");
        assert_eq!(bundle.prompt_version, "v1");
        assert_eq!(bundle.snapshot_hash, snapshot_hash(&snapshot).unwrap());
        assert!(
            bundle
                .content
                .contains("builtin.workflow_report_stage_result")
        );
        assert!(bundle.content.contains("implementation.v1"));
        assert!(bundle.content.contains("changed_files"));
        assert!(bundle.content.contains("commands_run"));
        assert!(bundle.content.contains("test_evidence"));
        assert!(bundle.content.contains("pr_ready"));
        assert!(
            bundle
                .content
                .contains("No unknown payload fields are accepted.")
        );
    }

    #[test]
    fn prompt_refs_and_asset_paths_use_prompt_pack_file_names() {
        let expected_prompt_assets = [
            (GithubIssueStage::Triage, "triage"),
            (GithubIssueStage::Planning, "plan"),
            (GithubIssueStage::Implementation, "implement"),
            (GithubIssueStage::PrSynthesis, "synthesize_pr"),
            (GithubIssueStage::CiRepair, "repair_ci"),
            (GithubIssueStage::ReviewResponse, "address_review"),
        ];

        for (stage, file_stem) in expected_prompt_assets {
            let snapshot = snapshot_for_stage(stage.clone());
            let bundle = render_stage_prompt(stage, &snapshot).unwrap();

            assert_eq!(
                bundle.prompt_ref,
                format!("github_issue_bugfix/v1/{file_stem}")
            );
            assert!(
                prompt_pack_dir().join(format!("{file_stem}.md")).is_file(),
                "missing expected prompt asset {file_stem}.md"
            );
        }

        for obsolete_file_name in [
            "planning.md",
            "implementation.md",
            "pr_synthesis.md",
            "ci_repair.md",
            "review_response.md",
        ] {
            assert!(
                !prompt_pack_dir().join(obsolete_file_name).exists(),
                "obsolete stage-slug prompt asset should not exist: {obsolete_file_name}"
            );
        }
    }

    #[test]
    fn prompt_schema_contract_is_owned_by_stage_schemas() {
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let prompts_source = fs::read_to_string(crate_dir.join("src/prompts.rs")).unwrap();
        let stage_schemas_source =
            fs::read_to_string(crate_dir.join("src/stage_schemas.rs")).unwrap();

        assert!(stage_schemas_source.contains("pub fn render_stage_result_schema_contract"));
        assert!(prompts_source.contains("render_stage_result_schema_contract("));
        assert!(
            !prompts_source.contains(
                "\"outcome\": \"completed | needs_human | gave_up | exhausted_turns | not_produced\""
            ),
            "prompts.rs must not hard-code the result envelope contract"
        );
        assert!(
            !prompts_source.contains("No unknown payload fields are accepted."),
            "prompts.rs must not hard-code payload validation text"
        );
    }

    #[test]
    fn planning_prompt_disallows_direct_github_writes() {
        let snapshot = planning_snapshot();
        let bundle = render_stage_prompt(GithubIssueStage::Planning, &snapshot).unwrap();

        assert!(bundle.content.contains("planning.v1"));
        assert!(
            bundle
                .content
                .contains("Do not call GitHub write tools directly.")
        );
        assert!(bundle.content.contains(
            "Return provider-write intent only; workflow provider actions perform GitHub writes."
        ));
        assert!(bundle.content.contains("files_to_inspect_or_change"));
        assert!(bundle.content.contains("test_strategy"));
    }

    #[test]
    fn prompt_content_hash_changes_when_prompt_file_changes() {
        let snapshot = implementation_snapshot();
        let bundle = render_stage_prompt(GithubIssueStage::Implementation, &snapshot).unwrap();
        let mut changed_content = bundle.content.clone();
        changed_content.push_str("\nPrompt asset mutation for hash contract.\n");

        assert_eq!(bundle.content_hash, sha256_hex(&bundle.content));
        assert_ne!(bundle.content_hash, sha256_hex(&changed_content));
    }
}
