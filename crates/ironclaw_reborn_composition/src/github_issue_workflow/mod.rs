//! GitHub issue workflow composition root.
//!
//! This module is the thin root of the GitHub issue workflow composition. It
//! owns only the values shared across two or more submodules — the adapter id,
//! the capability-id / capability-array / profile-id consts, the stage-thread
//! `kind` discriminator, and the `workflow_invalid_config` error helper — plus
//! the submodule declarations and the `pub(crate)`/`pub use` re-export wall that
//! lets external callers reach the workflow surface via
//! `crate::github_issue_workflow::<Item>`. All behavior lives in the submodules.

use ironclaw_github_issue_workflow::GithubIssueWorkflowError;

mod approval_policy;
mod capability_dispatcher;
mod capability_profiles;
mod config_source;
mod git_host;
mod github_port;
mod normalize;
mod spawn;
mod stage_result_sink;
mod stage_turn_submitter;
mod workspace_manager;

// Re-export wall: external callers reach these via
// `crate::github_issue_workflow::<Item>`.
// The capability-dispatcher trait/types are surfaced only for `test_support`
// (the production dispatcher path imports them directly from the submodule), so
// gate the re-export to its sole consumer to avoid an unused-import warning when
// `test-support` is off.
#[cfg(any(test, feature = "test-support"))]
pub(crate) use approval_policy::workflow_stage_loop_driver_grantee;
pub(crate) use approval_policy::{
    GithubIssueWorkflowStageApprovalGrantInput, GithubIssueWorkflowStageApprovalPolicyInput,
    ensure_github_issue_workflow_stage_approval_policies, workflow_stage_approval_capability_ids,
};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use capability_dispatcher::{
    GithubIssueWorkflowCapabilityDispatchError, GithubIssueWorkflowCapabilityDispatchRequest,
    GithubIssueWorkflowCapabilityDispatcher,
};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use capability_profiles::{
    GithubIssueWorkflowCapabilityProfile, allowed_capabilities_for_stage_profile_id,
    non_workflow_default_capability_profile, stage_capability_profiles,
    workflow_spawn_subagent_schema,
};
pub(crate) use capability_profiles::{
    GithubIssueWorkflowCapabilitySurfaceResolver, GithubIssueWorkflowSubagentDefinitionResolver,
    is_github_issue_workflow_context, planned_run_profile_resolver_with_stage_profiles,
    workflow_subagent_flavor_catalog,
};
pub(crate) use config_source::{
    project_metadata_github_issue_workflow_config_source,
    project_service_github_issue_workflow_project_access,
};
pub(crate) use github_port::IronClawGithubIssueWorkflowPort;
pub(crate) use spawn::{
    GithubIssueWorkflowRuntimeDeps, GithubIssueWorkflowRuntimeHandle, spawn_github_issue_workflow,
    test_only_empty_config_source, test_only_provider_account_ref,
    test_only_unconfigured_project_access, test_only_unconfigured_workspace_manager,
};
pub(crate) use stage_result_sink::{
    GITHUB_ISSUE_WORKFLOW_SHUTDOWN_TIMEOUT, WorkflowStageResultSinkSlot,
    insert_workflow_stage_result_handler,
};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use stage_turn_submitter::IronClawStageTurnSubmitter;
pub(crate) use stage_turn_submitter::workflow_stage_workspace_mount_view_from_thread_metadata;
pub(crate) use workspace_manager::runtime_workflow_workspace_manager;
#[cfg(any(test, feature = "test-support"))]
pub use workspace_manager::runtime_workflow_workspace_manager_for_test;

const WORKFLOW_ADAPTER_ID: &str = "github_issue_workflow";
const RESULT_SINK_CAPABILITY_ID: &str =
    ironclaw_host_runtime::WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID;
const SPAWN_SUBAGENT_CAPABILITY_ID: &str =
    ironclaw_loop_support::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID;
const SUBAGENT_RUN_PROFILE_ID: &str =
    ironclaw_reborn::planned_driver_factory::SUBAGENT_PLANNED_PROFILE_ID;

pub(crate) const GITHUB_BUG_TRIAGE_PROFILE_ID: &str = "github-bug-triage-v1";
pub(crate) const GITHUB_BUG_PLANNING_PROFILE_ID: &str = "github-bug-planning-v1";
pub(crate) const GITHUB_BUG_IMPLEMENTATION_PROFILE_ID: &str = "github-bug-implementation-v1";
pub(crate) const GITHUB_BUG_PR_SYNTHESIS_PROFILE_ID: &str = "github-bug-pr-synthesis-v1";
pub(crate) const GITHUB_BUG_CI_REPAIR_PROFILE_ID: &str = "github-bug-ci-repair-v1";
pub(crate) const GITHUB_BUG_REVIEW_RESPONSE_PROFILE_ID: &str = "github-bug-review-response-v1";
const GITHUB_SEARCH_ISSUES_CAPABILITY_ID: &str = "github.search_issues";
const GITHUB_GET_ISSUE_CAPABILITY_ID: &str = "github.get_issue";
const GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID: &str = "github.list_issue_comments";
const GITHUB_COMMENT_ISSUE_CAPABILITY_ID: &str = "github.comment_issue";
const GITHUB_LIST_PULL_REQUESTS_CAPABILITY_ID: &str = "github.list_pull_requests";
const GITHUB_CREATE_PULL_REQUEST_CAPABILITY_ID: &str = "github.create_pull_request";
const GITHUB_GET_PULL_REQUEST_CAPABILITY_ID: &str = "github.get_pull_request";
const GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID: &str = "github.list_pull_request_comments";
const GITHUB_GET_COMBINED_STATUS_CAPABILITY_ID: &str = "github.get_combined_status";
const GITHUB_GET_AUTHENTICATED_USER_CAPABILITY_ID: &str = "github.get_authenticated_user";
const WORKFLOW_GITHUB_CAPABILITY_IDS: &[&str] = &[
    GITHUB_SEARCH_ISSUES_CAPABILITY_ID,
    GITHUB_GET_ISSUE_CAPABILITY_ID,
    GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID,
    GITHUB_COMMENT_ISSUE_CAPABILITY_ID,
    GITHUB_LIST_PULL_REQUESTS_CAPABILITY_ID,
    GITHUB_CREATE_PULL_REQUEST_CAPABILITY_ID,
    GITHUB_GET_PULL_REQUEST_CAPABILITY_ID,
    GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID,
    GITHUB_GET_COMBINED_STATUS_CAPABILITY_ID,
    GITHUB_GET_AUTHENTICATED_USER_CAPABILITY_ID,
];

const READ_FILE_CAPABILITY_ID: &str = "builtin.read_file";
const WRITE_FILE_CAPABILITY_ID: &str = "builtin.write_file";
const APPLY_PATCH_CAPABILITY_ID: &str = "builtin.apply_patch";
const LIST_DIR_CAPABILITY_ID: &str = "builtin.list_dir";
const GREP_CAPABILITY_ID: &str = "builtin.grep";
const GLOB_CAPABILITY_ID: &str = "builtin.glob";
const SHELL_CAPABILITY_ID: &str = "builtin.shell";

const TRIAGE_PLANNING_CAPABILITIES: &[&str] = &[
    READ_FILE_CAPABILITY_ID,
    LIST_DIR_CAPABILITY_ID,
    GREP_CAPABILITY_ID,
    GLOB_CAPABILITY_ID,
    SPAWN_SUBAGENT_CAPABILITY_ID,
    RESULT_SINK_CAPABILITY_ID,
];

const IMPLEMENTATION_CAPABILITIES: &[&str] = &[
    READ_FILE_CAPABILITY_ID,
    WRITE_FILE_CAPABILITY_ID,
    APPLY_PATCH_CAPABILITY_ID,
    LIST_DIR_CAPABILITY_ID,
    GREP_CAPABILITY_ID,
    GLOB_CAPABILITY_ID,
    SHELL_CAPABILITY_ID,
    SPAWN_SUBAGENT_CAPABILITY_ID,
    RESULT_SINK_CAPABILITY_ID,
];

const PR_SYNTHESIS_CAPABILITIES: &[&str] = &[
    READ_FILE_CAPABILITY_ID,
    LIST_DIR_CAPABILITY_ID,
    GREP_CAPABILITY_ID,
    GLOB_CAPABILITY_ID,
    SHELL_CAPABILITY_ID,
    SPAWN_SUBAGENT_CAPABILITY_ID,
    RESULT_SINK_CAPABILITY_ID,
];

const WORKFLOW_SUBAGENT_CAPABILITIES: &[&str] = &[
    READ_FILE_CAPABILITY_ID,
    LIST_DIR_CAPABILITY_ID,
    GREP_CAPABILITY_ID,
    GLOB_CAPABILITY_ID,
];

#[cfg(any(test, feature = "test-support"))]
const NON_WORKFLOW_DEFAULT_CAPABILITIES: &[&str] = &[
    "builtin.echo",
    "builtin.time",
    "builtin.json",
    "builtin.http",
    "builtin.http.save",
    "builtin.memory_search",
    "builtin.memory_write",
    "builtin.profile_set",
    "builtin.memory_read",
    "builtin.memory_tree",
    SHELL_CAPABILITY_ID,
    READ_FILE_CAPABILITY_ID,
    WRITE_FILE_CAPABILITY_ID,
    LIST_DIR_CAPABILITY_ID,
    GLOB_CAPABILITY_ID,
    GREP_CAPABILITY_ID,
    APPLY_PATCH_CAPABILITY_ID,
    SPAWN_SUBAGENT_CAPABILITY_ID,
];

/// Metadata `kind` discriminator written onto a stage thread by
/// `stage_turn_submitter::stage_thread_metadata` and required by
/// `stage_result_sink::GithubWorkflowStageResultSink` when deriving the
/// authoritative stage identity. Shared so the writer and the reader cannot
/// drift — hence it lives in this root module rather than either submodule.
const GITHUB_ISSUE_WORKFLOW_STAGE_THREAD_KIND: &str = "github_issue_workflow_stage";

fn workflow_invalid_config(error: impl std::fmt::Display) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::InvalidConfig {
        reason: error.to_string(),
    }
}
