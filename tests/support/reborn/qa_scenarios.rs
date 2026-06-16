use std::collections::BTreeSet;

pub const QA_SCENARIOS: &[&str] = &[
    "three_step_time_write_read_summary",
    "session_continuity_write_read_append",
    "automation_heartbeat_smoke",
    "paused_cron_automation_smoke",
    "subagent_capability_smoke",
    "skill_discovery_smoke",
    "skill_invocation_smoke",
    "browser_integration_smoke",
    "local_browser_interaction_smoke",
    "mcp_discovery_smoke",
    "plugin_capability_smoke",
    "github_capability_smoke",
    "document_artifact_smoke",
    "spreadsheet_artifact_smoke",
    "presentation_artifact_smoke",
    "image_generation_smoke",
    "error_handling_smoke",
    "long_running_process_smoke",
    "repo_read_only_review_smoke",
    "approval_boundary_smoke",
    "patch_isolation_smoke",
    "cleanup_verification_smoke",
];

pub fn assert_all_covered(covered_scenarios: &[&str]) {
    let expected = QA_SCENARIOS.iter().copied().collect::<BTreeSet<_>>();
    let covered = covered_scenarios.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        expected, covered,
        "each pasted QA smoke scenario must be represented in Reborn e2e coverage"
    );
}
