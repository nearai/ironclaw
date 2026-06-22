#![cfg(all(feature = "github-issue-workflow-beta", feature = "test-support"))]

mod github_issue_workflow_capabilities {
    use std::collections::BTreeSet;

    use ironclaw_reborn_composition::test_support::{
        GithubIssueWorkflowCapabilityProfileForTest,
        github_issue_workflow_allowed_capabilities_for_profile_for_test,
        github_issue_workflow_capability_profiles_for_test,
        github_issue_workflow_default_capability_profile_for_test,
        github_issue_workflow_resolved_stage_profile_ids_for_test,
        github_issue_workflow_spawn_subagent_schema_for_test,
        github_issue_workflow_subagent_definition_profile_for_test,
    };

    const RESULT_SINK: &str = "builtin.workflow_report_stage_result";

    const GITHUB_WRITE_CAPABILITIES: &[&str] = &[
        "github.create_issue_comment",
        "github.comment_issue",
        "github.create_pull_request",
        "github.reply_pull_request_comment",
        "github.merge_pull_request",
    ];

    #[test]
    fn implementation_profile_contains_write_file_patch_shell_and_result_sink() {
        let profile = stage_profile("github-bug-implementation-v1");

        for capability in [
            "builtin.write_file",
            "builtin.apply_patch",
            "builtin.shell",
            RESULT_SINK,
        ] {
            assert!(
                profile.allowed_capabilities.contains(&capability),
                "{capability} must be visible in implementation stage profile"
            );
        }
    }

    #[test]
    fn planning_profile_excludes_write_file_patch_shell() {
        let profile = stage_profile("github-bug-planning-v1");

        for capability in ["builtin.write_file", "builtin.apply_patch", "builtin.shell"] {
            assert!(
                !profile.allowed_capabilities.contains(&capability),
                "{capability} must not be visible in planning stage profile"
            );
        }
        assert!(
            profile.allowed_capabilities.contains(&RESULT_SINK),
            "planning stage still needs the workflow result sink"
        );
    }

    #[test]
    fn all_stage_profiles_exclude_github_write_capabilities() {
        let profiles = github_issue_workflow_capability_profiles_for_test();
        let profile_ids: BTreeSet<&str> =
            profiles.iter().map(|profile| profile.profile_id).collect();

        assert_eq!(
            profile_ids,
            BTreeSet::from([
                "github-bug-triage-v1",
                "github-bug-planning-v1",
                "github-bug-implementation-v1",
                "github-bug-pr-synthesis-v1",
                "github-bug-ci-repair-v1",
                "github-bug-review-response-v1",
            ])
        );
        assert_eq!(
            github_issue_workflow_resolved_stage_profile_ids_for_test(),
            profile_ids,
            "stage profile ids must resolve through the planned run-profile registry"
        );

        for profile in profiles {
            assert_eq!(
                github_issue_workflow_allowed_capabilities_for_profile_for_test(profile.profile_id),
                Some(
                    profile
                        .allowed_capabilities
                        .iter()
                        .map(|capability| capability.to_string())
                        .collect()
                ),
                "{} must resolve through the composition capability surface resolver",
                profile.profile_id
            );
            for forbidden in GITHUB_WRITE_CAPABILITIES {
                assert!(
                    !profile.allowed_capabilities.contains(forbidden),
                    "{} must not expose GitHub write capability {forbidden}",
                    profile.profile_id
                );
            }
        }
    }

    #[test]
    fn result_sink_is_not_visible_in_non_workflow_default_profile() {
        let profile = github_issue_workflow_default_capability_profile_for_test();

        assert!(
            !profile.allowed_capabilities.contains(&RESULT_SINK),
            "workflow result sink must be visible only through workflow stage profiles"
        );
    }

    #[test]
    fn workflow_stage_profiles_do_not_allow_coder_subagent_flavor() {
        let schema = github_issue_workflow_spawn_subagent_schema_for_test();
        let enum_values = schema["properties"]["subagent_type"]["enum"]
            .as_array()
            .expect("spawn_subagent schema must publish subagent_type enum")
            .iter()
            .map(|value| value.as_str().expect("enum values must be strings"))
            .collect::<BTreeSet<_>>();

        assert_eq!(
            enum_values,
            BTreeSet::from(["explorer", "general", "planner"])
        );
        for flavor in ["general", "explorer", "planner"] {
            assert_eq!(
                github_issue_workflow_subagent_definition_profile_for_test(flavor),
                Some("reborn-planned-subagent".to_string())
            );
        }
        assert_eq!(
            github_issue_workflow_subagent_definition_profile_for_test("coder"),
            None
        );
        assert!(
            !enum_values.contains("coder"),
            "workflow stage spawn_subagent schema must omit coder"
        );
    }

    fn stage_profile(profile_id: &str) -> GithubIssueWorkflowCapabilityProfileForTest {
        github_issue_workflow_capability_profiles_for_test()
            .into_iter()
            .find(|profile| profile.profile_id == profile_id)
            .unwrap_or_else(|| panic!("missing stage profile {profile_id}"))
    }
}
