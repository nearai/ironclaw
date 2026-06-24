//! Stage approval-policy seeding for the GitHub issue workflow.
//!
//! Seeds the persistent approval policies that let the workflow's loop driver
//! dispatch the write/patch/shell capabilities a stage turn may request, and
//! resolves the loop-driver grantee extension id those grants are attributed to.
//! The capability-id consts these grants reference live in the parent module and
//! are surfaced here via `super::`.

use std::collections::BTreeSet;
use std::sync::Arc;

use ironclaw_approvals::{
    PersistentApprovalAction, PersistentApprovalPolicyInput, PersistentApprovalPolicyStore,
};
use ironclaw_github_issue_workflow::GithubIssueWorkflowError;
use ironclaw_host_api::{
    AgentId, CapabilityId, ExtensionId, GrantConstraints, InvocationId, Principal, ProjectId,
    ResourceScope, SystemServiceId, TenantId, ThreadId, UserId,
};
use ironclaw_loop_support::loop_driver_execution_extension_id;
use ironclaw_turns::{
    RunProfileRequest, RunProfileResolutionRequest, RunProfileResolver, TurnActor, TurnId,
    TurnRunId, TurnScope, run_profile::LoopRunContext,
};

use super::{
    APPLY_PATCH_CAPABILITY_ID, GITHUB_BUG_IMPLEMENTATION_PROFILE_ID, SHELL_CAPABILITY_ID,
    WORKFLOW_ADAPTER_ID, WRITE_FILE_CAPABILITY_ID,
    planned_run_profile_resolver_with_stage_profiles, workflow_invalid_config,
};

const WORKFLOW_STAGE_APPROVAL_CAPABILITY_IDS: &[&str] = &[
    WRITE_FILE_CAPABILITY_ID,
    APPLY_PATCH_CAPABILITY_ID,
    SHELL_CAPABILITY_ID,
];

pub(crate) fn workflow_stage_approval_capability_ids() -> &'static [&'static str] {
    WORKFLOW_STAGE_APPROVAL_CAPABILITY_IDS
}

pub(crate) struct GithubIssueWorkflowStageApprovalGrantInput {
    pub(crate) capability_id: CapabilityId,
    pub(crate) constraints: GrantConstraints,
}

pub(crate) struct GithubIssueWorkflowStageApprovalPolicyInput {
    pub(crate) tenant_id: TenantId,
    pub(crate) actor_user_id: UserId,
    pub(crate) agent_id: AgentId,
    pub(crate) project_id: Option<ProjectId>,
    pub(crate) grants: Vec<GithubIssueWorkflowStageApprovalGrantInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GithubIssueWorkflowStageApprovalPolicySeed {
    pub(crate) loop_driver_grantee: ExtensionId,
    pub(crate) capability_ids: BTreeSet<String>,
}

pub(crate) async fn ensure_github_issue_workflow_stage_approval_policies(
    policies: Arc<dyn PersistentApprovalPolicyStore>,
    input: GithubIssueWorkflowStageApprovalPolicyInput,
) -> Result<GithubIssueWorkflowStageApprovalPolicySeed, GithubIssueWorkflowError> {
    let loop_driver_grantee = workflow_stage_loop_driver_grantee(&input).await?;
    let approved_by = Principal::System(
        SystemServiceId::new(WORKFLOW_ADAPTER_ID).map_err(workflow_invalid_config)?,
    );
    let scope = ResourceScope {
        tenant_id: input.tenant_id,
        user_id: input.actor_user_id,
        agent_id: Some(input.agent_id),
        project_id: input.project_id,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let mut capability_ids = BTreeSet::new();
    for grant in input.grants {
        let capability_id = grant.capability_id;
        capability_ids.insert(capability_id.as_str().to_string());
        policies
            .allow(PersistentApprovalPolicyInput {
                scope: scope.clone(),
                action: PersistentApprovalAction::Dispatch,
                capability_id,
                grantee: Principal::Extension(loop_driver_grantee.clone()),
                approved_by: approved_by.clone(),
                constraints: grant.constraints,
                source_approval_request_id: None,
            })
            .await
            .map_err(|error| GithubIssueWorkflowError::Repository {
                reason: format!("workflow stage approval policy seed failed: {error}"),
            })?;
    }
    Ok(GithubIssueWorkflowStageApprovalPolicySeed {
        loop_driver_grantee,
        capability_ids,
    })
}

pub(crate) async fn workflow_stage_loop_driver_grantee(
    input: &GithubIssueWorkflowStageApprovalPolicyInput,
) -> Result<ExtensionId, GithubIssueWorkflowError> {
    let resolver = planned_run_profile_resolver_with_stage_profiles()
        .map_err(|error| workflow_invalid_config(error.to_string()))?;
    let requested_run_profile = RunProfileRequest::new(GITHUB_BUG_IMPLEMENTATION_PROFILE_ID)
        .map_err(workflow_invalid_config)?;
    let resolved = resolver
        .resolve_run_profile(
            RunProfileResolutionRequest::interactive_default()
                .with_requested_run_profile(requested_run_profile),
        )
        .await
        .map_err(|error| workflow_invalid_config(error.to_string()))?;
    let thread_id =
        ThreadId::new("github-issue-workflow-approval-seed").map_err(workflow_invalid_config)?;
    let scope = TurnScope::new_with_owner(
        input.tenant_id.clone(),
        Some(input.agent_id.clone()),
        input.project_id.clone(),
        thread_id,
        Some(input.actor_user_id.clone()),
    );
    let run_context = LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved)
        .with_actor(TurnActor::new(input.actor_user_id.clone()));
    loop_driver_execution_extension_id(&run_context).map_err(workflow_invalid_config)
}
