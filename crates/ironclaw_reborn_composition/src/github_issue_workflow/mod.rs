use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_approvals::{
    PersistentApprovalAction, PersistentApprovalPolicyInput, PersistentApprovalPolicyStore,
};
use ironclaw_github_issue_workflow::{
    AcceptStageResultInput, AcceptStageResultOutcome, GithubIssueStage, GithubIssueStageRunId,
    GithubIssueWorkflowConfigSource, GithubIssueWorkflowError, GithubIssueWorkflowPoller,
    GithubIssueWorkflowPollerConfig, GithubIssueWorkflowPollerPorts,
    GithubIssueWorkflowPollerWakeReceiver, GithubIssueWorkflowPollerWakeSender,
    GithubIssueWorkflowPort, GithubIssueWorkflowRepository, GithubIssueWorkflowRunId,
    GithubIssueWorkspaceSessionId, GithubProviderAccountRef, RecordWorkflowEventInput,
    StageCompletedPayload, StageTurnSubmitter, SubmitStageTurnOutcome, SubmitStageTurnRequest,
    WorkflowActorScope, WorkflowClock, WorkflowEventEnvelope, WorkflowEventSourceKind,
    WorkflowProjectAccess, WorkflowWorkerId, WorkflowWorkspaceManager, issue_binding_ref,
    stage_result_reported_key, validate_stage_result,
};
use ironclaw_host_api::{
    AgentId, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, CorrelationId,
    EffectKind, ExecutionContext, ExtensionId, GrantConstraints, InvocationId, MountView,
    NetworkPolicy, NetworkScheme, NetworkTargetPattern, Principal, ProjectId, ResourceScope,
    RuntimeKind, SystemServiceId, TenantId, ThreadId, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    ExecutingStageThread, FirstPartyCapabilityError, FirstPartyCapabilityHandler,
    FirstPartyCapabilityRegistry, FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
    ReportWorkflowStageResultInput, WorkflowStageResultAck, WorkflowStageResultSink,
    WorkflowStageResultSinkError, builtin_first_party_handlers_with_workflow_stage_result_sink,
};
#[cfg(any(test, feature = "test-support"))]
use ironclaw_loop_support::build_spawn_subagent_parameters_schema;
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    SpawnSubagentFlavorDescriptor, SubagentDefinition, SubagentDefinitionResolver, SubagentKindId,
    loop_driver_execution_extension_id,
};
use ironclaw_product_context::InboundClassification;
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent, MessageStatus,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadService,
    ThreadHistoryRequest, ThreadMessageId, ThreadScope,
};
use ironclaw_trust::TrustDecision;
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustProvenance};
use ironclaw_turns::{
    AcceptedMessageRef, IdempotencyKey, ProductTurnContext, ReplyTargetBindingRef,
    RunOriginAdapter, RunProfileRequest, RunProfileResolutionRequest, RunProfileResolver,
    SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError,
    TurnId, TurnRunId, TurnScope, TurnSurfaceType,
    run_profile::{
        CapabilitySurfaceProfileId, InMemoryRunProfileRegistry, InMemoryRunProfileResolver,
        LoopRunContext, RunProfileDefinition, RunProfileRegistryError,
    },
};
use serde_json::{Value as JsonValue, json};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use uuid::Uuid;

mod capability_dispatcher;
mod config_source;
mod git_host;
mod github_port;
mod normalize;
mod workspace_manager;

// Re-export wall: external callers reach these via
// `crate::github_issue_workflow::<Item>`.
// The capability-dispatcher trait/types are surfaced only for `test_support`
// (the production dispatcher path imports them directly from the submodule), so
// gate the re-export to its sole consumer to avoid an unused-import warning when
// `test-support` is off.
#[cfg(any(test, feature = "test-support"))]
pub(crate) use capability_dispatcher::{
    GithubIssueWorkflowCapabilityDispatchError, GithubIssueWorkflowCapabilityDispatchRequest,
    GithubIssueWorkflowCapabilityDispatcher,
};
pub(crate) use config_source::{
    project_metadata_github_issue_workflow_config_source,
    project_service_github_issue_workflow_project_access,
};
pub(crate) use github_port::IronClawGithubIssueWorkflowPort;
pub(crate) use workspace_manager::runtime_workflow_workspace_manager;
#[cfg(any(test, feature = "test-support"))]
pub use workspace_manager::runtime_workflow_workspace_manager_for_test;

use capability_dispatcher::HostRuntimeGithubIssueWorkflowCapabilityDispatcher;
use config_source::{EmptyGithubIssueWorkflowConfigSource, UnconfiguredWorkflowProjectAccess};
#[cfg(test)]
use config_source::{
    ProjectMetadataGithubIssueWorkflowConfigSource, ProjectServiceWorkflowProjectAccess,
};
#[cfg(test)]
use git_host::WorkflowGitRemoteConfig;
#[cfg(test)]
use workspace_manager::{
    RuntimeWorkflowWorkspaceManager, git_branch_component, workflow_workspace_host_path,
};
use workspace_manager::{UnconfiguredWorkflowWorkspaceManager, workflow_workspace_mount_view};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GithubIssueWorkflowCapabilityProfile {
    pub(crate) profile_id: &'static str,
    pub(crate) allowed_capabilities: &'static [&'static str],
}

const GITHUB_ISSUE_WORKFLOW_STAGE_PROFILES: &[GithubIssueWorkflowCapabilityProfile] = &[
    GithubIssueWorkflowCapabilityProfile {
        profile_id: GITHUB_BUG_TRIAGE_PROFILE_ID,
        allowed_capabilities: TRIAGE_PLANNING_CAPABILITIES,
    },
    GithubIssueWorkflowCapabilityProfile {
        profile_id: GITHUB_BUG_PLANNING_PROFILE_ID,
        allowed_capabilities: TRIAGE_PLANNING_CAPABILITIES,
    },
    GithubIssueWorkflowCapabilityProfile {
        profile_id: GITHUB_BUG_IMPLEMENTATION_PROFILE_ID,
        allowed_capabilities: IMPLEMENTATION_CAPABILITIES,
    },
    GithubIssueWorkflowCapabilityProfile {
        profile_id: GITHUB_BUG_PR_SYNTHESIS_PROFILE_ID,
        allowed_capabilities: PR_SYNTHESIS_CAPABILITIES,
    },
    GithubIssueWorkflowCapabilityProfile {
        profile_id: GITHUB_BUG_CI_REPAIR_PROFILE_ID,
        allowed_capabilities: IMPLEMENTATION_CAPABILITIES,
    },
    GithubIssueWorkflowCapabilityProfile {
        profile_id: GITHUB_BUG_REVIEW_RESPONSE_PROFILE_ID,
        allowed_capabilities: PR_SYNTHESIS_CAPABILITIES,
    },
];

#[cfg(any(test, feature = "test-support"))]
const NON_WORKFLOW_DEFAULT_PROFILE: GithubIssueWorkflowCapabilityProfile =
    GithubIssueWorkflowCapabilityProfile {
        profile_id: ironclaw_reborn::planned_driver_factory::PLANNED_DEFAULT_PROFILE_ID,
        allowed_capabilities: NON_WORKFLOW_DEFAULT_CAPABILITIES,
    };

pub(crate) fn stage_capability_profile_id(stage: &GithubIssueStage) -> &'static str {
    match stage {
        GithubIssueStage::Triage => GITHUB_BUG_TRIAGE_PROFILE_ID,
        GithubIssueStage::Planning => GITHUB_BUG_PLANNING_PROFILE_ID,
        GithubIssueStage::Implementation => GITHUB_BUG_IMPLEMENTATION_PROFILE_ID,
        GithubIssueStage::PrSynthesis => GITHUB_BUG_PR_SYNTHESIS_PROFILE_ID,
        GithubIssueStage::CiRepair => GITHUB_BUG_CI_REPAIR_PROFILE_ID,
        GithubIssueStage::ReviewResponse => GITHUB_BUG_REVIEW_RESPONSE_PROFILE_ID,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn stage_capability_profiles() -> &'static [GithubIssueWorkflowCapabilityProfile] {
    GITHUB_ISSUE_WORKFLOW_STAGE_PROFILES
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn non_workflow_default_capability_profile() -> GithubIssueWorkflowCapabilityProfile {
    NON_WORKFLOW_DEFAULT_PROFILE
}

fn stage_profile_for_surface_id(
    profile_id: &CapabilitySurfaceProfileId,
) -> Option<&'static GithubIssueWorkflowCapabilityProfile> {
    GITHUB_ISSUE_WORKFLOW_STAGE_PROFILES
        .iter()
        .find(|profile| profile.profile_id == profile_id.as_str())
}

fn capability_allow_set_for_profile(
    profile: &GithubIssueWorkflowCapabilityProfile,
) -> Result<CapabilityAllowSet, CapabilityResolveError> {
    let ids = profile.allowed_capabilities.iter().map(|capability| {
        CapabilityId::new(*capability).map_err(|reason| {
            CapabilityResolveError::internal(format!(
                "invalid static GitHub issue workflow capability id {capability}: {reason}"
            ))
        })
    });
    ids.collect::<Result<Vec<_>, _>>()
        .map(CapabilityAllowSet::allowlist)
}

pub(crate) fn allowed_capabilities_for_stage_profile_id(
    profile_id: &CapabilitySurfaceProfileId,
) -> Result<Option<CapabilityAllowSet>, CapabilityResolveError> {
    stage_profile_for_surface_id(profile_id)
        .map(capability_allow_set_for_profile)
        .transpose()
}

pub(crate) fn allowed_capabilities_for_workflow_subagent_profile_id(
    profile_id: &CapabilitySurfaceProfileId,
) -> Result<Option<CapabilityAllowSet>, CapabilityResolveError> {
    if profile_id.as_str()
        != ironclaw_reborn::planned_driver_factory::SUBAGENT_CAPABILITY_SURFACE_PROFILE_ID
    {
        return Ok(None);
    }
    capability_allow_set_for_ids(WORKFLOW_SUBAGENT_CAPABILITIES).map(Some)
}

fn capability_allow_set_for_ids(
    capability_ids: &[&str],
) -> Result<CapabilityAllowSet, CapabilityResolveError> {
    let ids = capability_ids.iter().map(|capability| {
        CapabilityId::new(*capability).map_err(|reason| {
            CapabilityResolveError::internal(format!(
                "invalid static GitHub issue workflow capability id {capability}: {reason}"
            ))
        })
    });
    ids.collect::<Result<Vec<_>, _>>()
        .map(CapabilityAllowSet::allowlist)
}

pub(crate) struct GithubIssueWorkflowCapabilitySurfaceResolver {
    inner: Arc<dyn CapabilitySurfaceProfileResolver>,
}

impl GithubIssueWorkflowCapabilitySurfaceResolver {
    pub(crate) fn new(inner: Arc<dyn CapabilitySurfaceProfileResolver>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl CapabilitySurfaceProfileResolver for GithubIssueWorkflowCapabilitySurfaceResolver {
    async fn resolve(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        if is_github_issue_workflow_context(run_context)
            && let Some(allow_set) = allowed_capabilities_for_workflow_subagent_profile_id(
                &run_context
                    .resolved_run_profile
                    .capability_surface_profile_id,
            )?
        {
            return Ok(allow_set);
        }
        if let Some(allow_set) = allowed_capabilities_for_stage_profile_id(
            &run_context
                .resolved_run_profile
                .capability_surface_profile_id,
        )? {
            return Ok(allow_set);
        }
        self.inner.resolve(run_context).await
    }
}

pub(crate) fn is_github_issue_workflow_context(run_context: &LoopRunContext) -> bool {
    run_context
        .product_context
        .as_ref()
        .and_then(|context| context.adapter.as_ref())
        .is_some_and(|adapter| adapter.as_ref() == WORKFLOW_ADAPTER_ID)
}

pub(crate) fn workflow_subagent_flavor_catalog() -> Vec<SpawnSubagentFlavorDescriptor> {
    [
        (
            "general",
            "read-only file exploration (read_file, list_dir, grep)",
        ),
        (
            "explorer",
            "read + glob over filesystem (read_file, list_dir, grep, glob)",
        ),
        (
            "planner",
            "read codebase + planning context, returns a structured implementation plan \
             (read_file, list_dir, grep, glob)",
        ),
    ]
    .into_iter()
    .map(|(id, summary)| SpawnSubagentFlavorDescriptor {
        id: SubagentKindId::new(id)
            .expect("static workflow subagent flavor id is a valid SubagentKindId"),
        summary: summary.to_string(),
    })
    .collect()
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn workflow_spawn_subagent_schema() -> serde_json::Value {
    build_spawn_subagent_parameters_schema(&workflow_subagent_flavor_catalog())
}

pub(crate) fn planned_run_profile_resolver_with_stage_profiles()
-> Result<InMemoryRunProfileResolver, RunProfileRegistryError> {
    let mut registry = InMemoryRunProfileRegistry::with_builtin_profiles();
    ironclaw_reborn::planned_driver_factory::register_default_planned_profile(&mut registry)?;
    ironclaw_reborn::planned_driver_factory::register_subagent_planned_profile(&mut registry)?;
    register_stage_run_profiles(&mut registry)?;
    let implicit_default = ironclaw_reborn::planned_driver_factory::planned_default_profile_id()
        .map_err(invalid_run_profile)?;
    Ok(InMemoryRunProfileResolver::new_with_implicit_default(
        registry,
        implicit_default,
    ))
}

fn register_stage_run_profiles(
    registry: &mut InMemoryRunProfileRegistry,
) -> Result<(), RunProfileRegistryError> {
    let descriptor = ironclaw_reborn::planned_driver_factory::planned_driver_descriptor()
        .map_err(invalid_run_profile)?;
    let checkpoint_schema_id =
        ironclaw_reborn::planned_driver_factory::planned_driver_checkpoint_schema_id()
            .map_err(invalid_run_profile)?;
    let checkpoint_schema_version =
        ironclaw_reborn::planned_driver_factory::planned_driver_checkpoint_schema_version();

    for profile in GITHUB_ISSUE_WORKFLOW_STAGE_PROFILES {
        let profile_id =
            ironclaw_turns::RunProfileId::new(profile.profile_id).map_err(invalid_run_profile)?;
        let capability_surface_profile_id =
            CapabilitySurfaceProfileId::new(profile.profile_id).map_err(invalid_run_profile)?;
        registry.register(
            RunProfileDefinition::interactive_like(
                profile_id,
                descriptor.clone(),
                checkpoint_schema_id.clone(),
                checkpoint_schema_version,
                capability_surface_profile_id,
            )
            // Headless: the workflow poller drives these stage turns with no human
            // attached, so a budget-cap crossing must degrade to a recoverable
            // error rather than open an approval gate that strands the run.
            .with_non_interactive_budget(),
        )?;
    }
    Ok(())
}

fn invalid_run_profile(reason: String) -> RunProfileRegistryError {
    RunProfileRegistryError::InvalidProfile { reason }
}

#[cfg(test)]
mod stage_profile_budget_tests {
    use ironclaw_turns::run_profile::{BudgetApprovalMode, RunProfileResolutionRequest};
    use ironclaw_turns::{RunProfileRequest, RunProfileResolver};

    #[tokio::test]
    async fn github_workflow_stage_profiles_are_non_interactive_for_budget() {
        // The poller drives these stage turns with no human attached, so each
        // resolved profile must be NonInteractive — a budget-cap crossing then
        // degrades to a recoverable error instead of stranding the run on an
        // unanswerable approval gate.
        let resolver = super::planned_run_profile_resolver_with_stage_profiles()
            .expect("stage profile resolver");
        for profile in super::GITHUB_ISSUE_WORKFLOW_STAGE_PROFILES {
            let requested = RunProfileRequest::new(profile.profile_id).expect("profile id");
            let resolved = resolver
                .resolve_run_profile(
                    RunProfileResolutionRequest::interactive_default()
                        .with_requested_run_profile(requested),
                )
                .await
                .expect("resolve stage profile");
            assert_eq!(
                resolved.budget_approval_mode,
                BudgetApprovalMode::NonInteractive,
                "github stage profile `{}` must be non-interactive for budget approval",
                profile.profile_id
            );
        }
    }
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

#[derive(Default)]
pub(crate) struct GithubIssueWorkflowSubagentDefinitionResolver;

#[async_trait]
impl SubagentDefinitionResolver for GithubIssueWorkflowSubagentDefinitionResolver {
    async fn resolve_kind(
        &self,
        kind: &SubagentKindId,
    ) -> Result<Option<SubagentDefinition>, ironclaw_turns::run_profile::AgentLoopHostError> {
        if !matches!(kind.as_str(), "general" | "explorer" | "planner") {
            return Ok(None);
        }
        let requested_run_profile =
            RunProfileRequest::new(SUBAGENT_RUN_PROFILE_ID).map_err(|reason| {
                ironclaw_turns::run_profile::AgentLoopHostError::new(
                    ironclaw_turns::run_profile::AgentLoopHostErrorKind::Internal,
                    reason,
                )
            })?;
        Ok(Some(SubagentDefinition {
            subagent_kind: kind.clone(),
            allow_nesting: false,
            requested_run_profile,
        }))
    }
}

pub(crate) fn insert_workflow_stage_result_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    trigger_repository: Arc<dyn ironclaw_triggers::TriggerRepository>,
    workflow_stage_result_sink_slot: Arc<WorkflowStageResultSinkSlot>,
) -> Result<(), ironclaw_host_api::HostApiError> {
    let capability_id = CapabilityId::new(RESULT_SINK_CAPABILITY_ID)?;
    let workflow_stage_result_sink: Arc<dyn WorkflowStageResultSink> =
        workflow_stage_result_sink_slot;
    let workflow_registry = builtin_first_party_handlers_with_workflow_stage_result_sink(
        trigger_repository,
        workflow_stage_result_sink,
    )?;
    let handler = workflow_registry.get(&capability_id).ok_or_else(|| {
        ironclaw_host_api::HostApiError::InvariantViolation {
            reason: format!(
                "workflow stage result helper did not register {RESULT_SINK_CAPABILITY_ID}"
            ),
        }
    })?;
    registry.insert_handler(
        capability_id,
        Arc::new(DelegatingWorkflowStageResultHandler { inner: handler }),
    );
    Ok(())
}

pub(crate) const GITHUB_ISSUE_WORKFLOW_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct WorkflowStageResultSinkSlot {
    inner: OnceLock<Arc<dyn WorkflowStageResultSink>>,
}

impl WorkflowStageResultSinkSlot {
    pub(crate) fn new() -> Self {
        Self {
            inner: OnceLock::new(),
        }
    }

    pub(crate) fn set(
        &self,
        sink: Arc<dyn WorkflowStageResultSink>,
    ) -> Result<(), Arc<dyn WorkflowStageResultSink>> {
        self.inner.set(sink)
    }
}

impl Default for WorkflowStageResultSinkSlot {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WorkflowStageResultSink for WorkflowStageResultSinkSlot {
    async fn report_stage_result(
        &self,
        executing_thread: ExecutingStageThread,
        input: ReportWorkflowStageResultInput,
    ) -> Result<WorkflowStageResultAck, WorkflowStageResultSinkError> {
        let Some(sink) = self.inner.get().cloned() else {
            return Err(WorkflowStageResultSinkError::Unavailable);
        };
        sink.report_stage_result(executing_thread, input).await
    }
}

struct DelegatingWorkflowStageResultHandler {
    inner: Arc<dyn FirstPartyCapabilityHandler>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for DelegatingWorkflowStageResultHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        self.inner.dispatch(request).await
    }
}

/// Metadata `kind` discriminator written onto a stage thread by
/// [`stage_thread_metadata`] and required by [`GithubWorkflowStageResultSink`]
/// when deriving the authoritative stage identity. Shared so the writer and the
/// reader cannot drift.
const GITHUB_ISSUE_WORKFLOW_STAGE_THREAD_KIND: &str = "github_issue_workflow_stage";

/// The authoritative stage identity the host derives from the trusted executing
/// thread's metadata. The model never supplies these — they are read back from
/// the thread the stage turn was dispatched into.
#[derive(serde::Deserialize)]
struct StageThreadBinding {
    kind: String,
    workflow_run_id: String,
    stage_run_id: String,
    stage: String,
}

pub(crate) struct GithubWorkflowStageResultSink {
    repository: Arc<dyn GithubIssueWorkflowRepository>,
    thread_service: Arc<dyn SessionThreadService>,
    default_agent_id: AgentId,
    // Required (not Option) per architecture.md rule #2: production always wires
    // a real sender, and tests construct a throwaway one via
    // `GithubIssueWorkflowPollerWakeReceiver::channel().0`. Fired right after a
    // StageCompleted event is recorded so the poller re-ticks the affected run
    // immediately rather than after a full poll interval.
    poller_wake: GithubIssueWorkflowPollerWakeSender,
}

impl GithubWorkflowStageResultSink {
    pub(crate) fn new(
        repository: Arc<dyn GithubIssueWorkflowRepository>,
        thread_service: Arc<dyn SessionThreadService>,
        default_agent_id: AgentId,
        poller_wake: GithubIssueWorkflowPollerWakeSender,
    ) -> Self {
        Self {
            repository,
            thread_service,
            default_agent_id,
            poller_wake,
        }
    }
}

#[async_trait]
impl WorkflowStageResultSink for GithubWorkflowStageResultSink {
    async fn report_stage_result(
        &self,
        executing_thread: ExecutingStageThread,
        input: ReportWorkflowStageResultInput,
    ) -> Result<WorkflowStageResultAck, WorkflowStageResultSinkError> {
        // The host stamps the executing thread scope. An absent thread id means
        // the result tool was invoked outside any stage turn — unauthenticated.
        let Some(thread_id) = executing_thread.scope.thread_id.clone() else {
            debug!("workflow stage result rejected: executing thread id is absent");
            return Err(WorkflowStageResultSinkError::MismatchedBinding);
        };

        // Reconstruct the thread scope EXACTLY as IronClawStageTurnSubmitter::
        // thread_scope wrote it, sourced from the trusted executing scope, so
        // read_thread's exact-scope ownership check matches the write side.
        let executing_scope = &executing_thread.scope;
        let thread_scope = ThreadScope {
            tenant_id: executing_scope.tenant_id.clone(),
            agent_id: executing_scope
                .agent_id
                .clone()
                .unwrap_or_else(|| self.default_agent_id.clone()),
            project_id: executing_scope.project_id.clone(),
            owner_user_id: Some(executing_scope.user_id.clone()),
            mission_id: executing_scope.mission_id.clone(),
        };

        let record = self
            .thread_service
            .read_thread(ThreadHistoryRequest {
                scope: thread_scope,
                thread_id: thread_id.clone(),
            })
            .await
            .map_err(stage_result_thread_error)?;

        // Derive the AUTHORITATIVE stage identity from the trusted, host-written
        // thread metadata — never from the model-supplied input fields.
        let Some(metadata_json) = record.metadata_json else {
            debug!("workflow stage result rejected: executing thread carries no binding metadata");
            return Err(WorkflowStageResultSinkError::MismatchedBinding);
        };
        let binding: StageThreadBinding = serde_json::from_str(&metadata_json).map_err(|_| {
            debug!(
                "workflow stage result rejected: executing thread metadata is not a stage binding"
            );
            WorkflowStageResultSinkError::MismatchedBinding
        })?;
        if binding.kind != GITHUB_ISSUE_WORKFLOW_STAGE_THREAD_KIND {
            debug!(
                "workflow stage result rejected: executing thread is not a github issue workflow stage"
            );
            return Err(WorkflowStageResultSinkError::MismatchedBinding);
        }
        let workflow_run_id = GithubIssueWorkflowRunId::from_trusted(binding.workflow_run_id)
            .map_err(stage_result_invalid_input)?;
        let stage_run_id = GithubIssueStageRunId::from_trusted(binding.stage_run_id)
            .map_err(stage_result_invalid_input)?;
        let stage = serde_json::from_value::<GithubIssueStage>(JsonValue::String(binding.stage))
            .map_err(|error| WorkflowStageResultSinkError::InvalidInput {
                reason: format!("invalid stage in executing thread binding: {error}"),
            })?;

        // Validate the model-supplied wire fields and cross-check them against
        // the authoritative identity (defense in depth + clearer errors). The
        // completion_nonce is deliberately NOT checked: it is never injected
        // into a stage prompt, so it carries no authority — the thread binding
        // is the authority.
        // turn_run_id is non-authoritative (the host binds via the executing
        // thread); validate its FORMAT only when the model bothered to send it.
        if let Some(turn_run_id) = input.turn_run_id.as_deref() {
            TurnRunId::parse(turn_run_id).map_err(|error| {
                WorkflowStageResultSinkError::InvalidInput {
                    reason: format!("invalid turn_run_id: {error}"),
                }
            })?;
        }
        let input_stage =
            serde_json::from_value::<GithubIssueStage>(JsonValue::String(input.stage.clone()))
                .map_err(|error| WorkflowStageResultSinkError::InvalidInput {
                    reason: format!("invalid stage: {error}"),
                })?;
        // The input schema no longer requires the model to supply
        // workflow_run_id/stage_run_id — it has no authoritative source for them
        // (they are not injected into any stage prompt). Cross-check them against
        // the thread-derived authoritative ids ONLY when present; a
        // present-but-wrong id is still a hard MismatchedBinding (defense in
        // depth). `stage` is always cross-checked.
        let workflow_run_mismatch = input
            .workflow_run_id
            .as_deref()
            .is_some_and(|value| value != workflow_run_id.as_str());
        let stage_run_mismatch = input
            .stage_run_id
            .as_deref()
            .is_some_and(|value| value != stage_run_id.as_str());
        if workflow_run_mismatch || stage_run_mismatch || input_stage != stage {
            debug!(
                "workflow stage result rejected: model-supplied identity does not match the executing thread binding"
            );
            return Err(WorkflowStageResultSinkError::MismatchedBinding);
        }

        let validated =
            validate_stage_result(stage, &input.schema_version, input.result).map_err(|error| {
                WorkflowStageResultSinkError::ValidationFailed {
                    reason: error.to_string(),
                }
            })?;
        let result = serde_json::to_value(&validated.envelope).map_err(|error| {
            WorkflowStageResultSinkError::InvalidInput {
                reason: format!("validated stage result could not be serialized: {error}"),
            }
        })?;
        let now = Utc::now();
        // `input.stage_run_id` is now optional; the ack reports the authoritative
        // (thread-derived) stage run id, not the model-supplied value.
        let ack_stage_run_id = stage_run_id.as_str().to_string();

        debug!(
            workflow_run_id = workflow_run_id.as_str(),
            stage_run_id = stage_run_id.as_str(),
            "workflow stage result bound to executing thread; accepting"
        );

        match self
            .repository
            .accept_stage_result(AcceptStageResultInput {
                workflow_run_id: workflow_run_id.clone(),
                stage_run_id: stage_run_id.clone(),
                result: result.clone(),
                now,
            })
            .await
            .map_err(stage_result_repository_error)?
        {
            AcceptStageResultOutcome::Accepted { run } => {
                self.repository
                    .record_workflow_event(RecordWorkflowEventInput {
                        workflow_run_id,
                        workflow_event_type:
                            ironclaw_github_issue_workflow::GithubIssueWorkflowEventType::StageCompleted,
                        envelope: WorkflowEventEnvelope {
                            source_kind: WorkflowEventSourceKind::WorkflowInternal,
                            source_delivery_id: None,
                            provider: issue_binding_ref(&run.issue_ref).provider_ref,
                            observed_at: now,
                            provider_updated_at: None,
                            idempotency_key: stage_result_reported_key(
                                &stage_run_id,
                                &validated.schema_version,
                            ),
                            payload_schema: "stage.completed.v1".to_string(),
                            payload: serde_json::to_value(StageCompletedPayload {
                                stage_run_id,
                                stage: validated.stage,
                                schema_version: validated.schema_version,
                                result,
                            })
                            .map_err(|error| WorkflowStageResultSinkError::InvalidInput {
                                reason: format!(
                                    "stage completed workflow event could not be serialized: {error}"
                                ),
                            })?,
                        },
                    })
                    .await
                    .map_err(stage_result_repository_error)?;
                // Wake the poller so it re-ticks this run at the stage boundary
                // immediately instead of waiting up to a full poll interval.
                // Best-effort/edge-triggered: the interval fallback still covers
                // a dropped wake.
                self.poller_wake.wake();
                Ok(WorkflowStageResultAck {
                    accepted: true,
                    duplicate: false,
                    stage_run_id: ack_stage_run_id,
                })
            }
            AcceptStageResultOutcome::NotActiveStage { .. } => {
                Err(WorkflowStageResultSinkError::StageNotActive)
            }
            AcceptStageResultOutcome::Terminal => Err(WorkflowStageResultSinkError::StageNotActive),
        }
    }
}

fn stage_result_thread_error(error: SessionThreadError) -> WorkflowStageResultSinkError {
    match error {
        // The executing thread does not exist under the reconstructed scope, or
        // exists under a different scope: the result tool is not bound to the
        // stage it claims to complete.
        SessionThreadError::UnknownThread { .. }
        | SessionThreadError::ThreadScopeMismatch { .. } => {
            WorkflowStageResultSinkError::MismatchedBinding
        }
        // Backend/serialization faults are transient infrastructure errors, not
        // a binding decision.
        _ => WorkflowStageResultSinkError::Unavailable,
    }
}

fn stage_result_invalid_input(error: GithubIssueWorkflowError) -> WorkflowStageResultSinkError {
    WorkflowStageResultSinkError::InvalidInput {
        reason: error.to_string(),
    }
}

fn stage_result_repository_error(error: GithubIssueWorkflowError) -> WorkflowStageResultSinkError {
    match error {
        GithubIssueWorkflowError::InvalidId { .. }
        | GithubIssueWorkflowError::InvalidConfig { .. } => {
            WorkflowStageResultSinkError::InvalidInput {
                reason: error.to_string(),
            }
        }
        GithubIssueWorkflowError::PolicyDenied { .. } | GithubIssueWorkflowError::Policy { .. } => {
            WorkflowStageResultSinkError::MismatchedBinding
        }
        GithubIssueWorkflowError::ProviderRead { .. }
        | GithubIssueWorkflowError::ProviderRateLimited { .. }
        | GithubIssueWorkflowError::Repository { .. } => WorkflowStageResultSinkError::Unavailable,
    }
}

#[cfg(test)]
mod github_issue_workflow_stage_result_sink_tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex as StdMutex};

    use async_trait::async_trait;
    use ironclaw_github_issue_workflow::{
        AdvanceWorkflowRunInput, ClaimRunnableWorkflowRunsInput, CreateDraftPullRequestInput,
        CreateIssueCommentInput, CreateOrGetWorkflowRunInput, CreateOrGetWorkflowRunOutcome,
        CreateStageRunInput, GetAuthenticatedWorkflowActorInput, GithubActorSnapshot,
        GithubCommentRef, GithubIssueCommentSnapshot, GithubIssueRef, GithubIssueStage,
        GithubIssueWorkflowError, GithubIssueWorkflowEventType, GithubIssueWorkflowMode,
        GithubIssueWorkflowPolicy, GithubIssueWorkflowPolicyPorts,
        GithubIssueWorkflowPollerWakeReceiver, GithubIssueWorkflowPollerWakeSender,
        GithubIssueWorkflowRepository, GithubIssueWorkflowRun, GithubIssueWorkflowRunKey,
        GithubIssueWorkspaceSession, GithubIssueWorkspaceSessionId, GithubProviderAccountRef,
        GithubPullRequestRef, GithubPullRequestSnapshot, GithubRepositorySelector,
        InMemoryGithubIssueWorkflowRepository, ListIssueCommentsInput, ListPullRequestsInput,
        ListWorkflowEventsAfterInput, PrepareWorkflowWorkspaceOutcome,
        PrepareWorkflowWorkspaceRequest, PublishWorkflowWorkspaceOutcome,
        PublishWorkflowWorkspaceRequest, StageTurnSubmitter, SubmitStageTurnOutcome,
        SubmitStageTurnRequest, TransitionOutcome, WorkflowClock, WorkflowEventSourceKind,
        WorkflowProjectAccess, WorkflowProjectAccessRequest, WorkflowRunTransition,
        WorkflowWorkerId, WorkflowWorkspaceManager, WorkflowWorkspaceMountRef,
        WorkflowWorkspaceRef,
    };
    use ironclaw_host_api::{
        AgentId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
    };
    use ironclaw_host_runtime::{
        ExecutingStageThread, ReportWorkflowStageResultInput, WorkflowStageResultSink,
    };
    use ironclaw_threads::{
        EnsureThreadRequest, InMemorySessionThreadService, SessionThreadService, ThreadScope,
    };
    use ironclaw_turns::{TurnRunId, TurnScope};
    use serde_json::json;
    use tokio::sync::Mutex;

    use super::{GITHUB_ISSUE_WORKFLOW_STAGE_THREAD_KIND, GithubWorkflowStageResultSink};

    fn sink_tenant() -> TenantId {
        TenantId::new("tenant-stage-result-sink").unwrap()
    }

    fn sink_user() -> UserId {
        UserId::new("user-stage-result-sink").unwrap()
    }

    fn sink_agent() -> AgentId {
        AgentId::new("agent-stage-result-sink").unwrap()
    }

    fn sink_project() -> ProjectId {
        ProjectId::new("project-stage-result-sink").unwrap()
    }

    /// A throwaway wake sender for sink tests that do not assert on the wake.
    /// The receiver is dropped immediately; `wake()` is a no-op `notify_one`,
    /// which is safe on a disconnected `Notify`.
    fn test_poller_wake() -> GithubIssueWorkflowPollerWakeSender {
        GithubIssueWorkflowPollerWakeReceiver::channel().0
    }

    fn stage_thread_id(
        workflow_run_id: &ironclaw_github_issue_workflow::GithubIssueWorkflowRunId,
        stage_run_id: &ironclaw_github_issue_workflow::GithubIssueStageRunId,
    ) -> ThreadId {
        ThreadId::new(format!(
            "github-issue-workflow:{}:stage:{}",
            workflow_run_id.as_str(),
            stage_run_id.as_str()
        ))
        .unwrap()
    }

    fn stage_thread_scope() -> ThreadScope {
        ThreadScope {
            tenant_id: sink_tenant(),
            agent_id: sink_agent(),
            project_id: Some(sink_project()),
            owner_user_id: Some(sink_user()),
            mission_id: None,
        }
    }

    /// Builds the trusted executing-thread scope the host would stamp for a turn
    /// running inside the stage thread of `(workflow_run_id, stage_run_id)`. It
    /// reconstructs to exactly `stage_thread_scope()`.
    fn executing_thread_for(
        workflow_run_id: &ironclaw_github_issue_workflow::GithubIssueWorkflowRunId,
        stage_run_id: &ironclaw_github_issue_workflow::GithubIssueStageRunId,
    ) -> ExecutingStageThread {
        ExecutingStageThread {
            scope: ResourceScope {
                tenant_id: sink_tenant(),
                user_id: sink_user(),
                agent_id: Some(sink_agent()),
                project_id: Some(sink_project()),
                mission_id: None,
                thread_id: Some(stage_thread_id(workflow_run_id, stage_run_id)),
                invocation_id: InvocationId::new(),
            },
        }
    }

    /// Creates the stage thread (with the `kind = github_issue_workflow_stage`
    /// binding metadata) that the sink reads to derive authoritative identity.
    async fn seed_stage_thread(
        thread_service: &InMemorySessionThreadService,
        workflow_run_id: &ironclaw_github_issue_workflow::GithubIssueWorkflowRunId,
        stage_run_id: &ironclaw_github_issue_workflow::GithubIssueStageRunId,
        stage: GithubIssueStage,
    ) {
        let metadata = serde_json::to_string(&json!({
            "kind": GITHUB_ISSUE_WORKFLOW_STAGE_THREAD_KIND,
            "workflow_run_id": workflow_run_id.as_str(),
            "stage_run_id": stage_run_id.as_str(),
            "stage": stage_name(&stage),
        }))
        .unwrap();
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: stage_thread_scope(),
                thread_id: Some(stage_thread_id(workflow_run_id, stage_run_id)),
                created_by_actor_id: sink_user().as_str().to_string(),
                title: Some("github issue workflow stage".to_string()),
                metadata_json: Some(metadata),
            })
            .await
            .expect("seed stage thread");
    }

    #[tokio::test]
    async fn stage_result_sink_accepts_and_records_event_for_matching_executing_thread() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Triage).await;
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_stage_thread(
            &thread_service,
            &workflow_run_id,
            &stage_run_id,
            GithubIssueStage::Triage,
        )
        .await;
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );

        let ack = sink
            .report_stage_result(
                executing_thread_for(&workflow_run_id, &stage_run_id),
                ReportWorkflowStageResultInput {
                    workflow_run_id: Some(workflow_run_id.as_str().to_string()),
                    stage_run_id: Some(stage_run_id.as_str().to_string()),
                    turn_run_id: Some(TurnRunId::new().to_string()),
                    stage: "triage".to_string(),
                    schema_version: "triage.v1".to_string(),
                    completion_nonce: Some("nonce-triage".to_string()),
                    result: json!({
                        "outcome": "completed",
                        "summary": "triage completed",
                        "evidence": [],
                        "next_actions": [],
                        "payload": {
                            "is_reproducible": true,
                            "suspected_area": "composition sink",
                            "risk": "medium",
                            "recommended_next_stage": "planning"
                        }
                    }),
                },
            )
            .await
            .expect("stage result should be accepted");

        assert!(ack.accepted);
        assert!(!ack.duplicate);
        assert_eq!(ack.stage_run_id, stage_run_id.as_str());

        let events = repository
            .list_workflow_events_after(ListWorkflowEventsAfterInput {
                workflow_run_id,
                after_sequence: 0,
                limit: 10,
            })
            .await
            .expect("list workflow events");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].workflow_event_type,
            GithubIssueWorkflowEventType::StageCompleted
        );
        assert_eq!(
            events[0].source_kind,
            WorkflowEventSourceKind::WorkflowInternal
        );
        assert_eq!(events[0].payload_schema, "stage.completed.v1");
        assert_eq!(events[0].payload["schema_version"], "triage.v1");
        assert_eq!(
            events[0].payload["result"]["payload"]["is_reproducible"],
            true
        );
    }

    #[tokio::test]
    async fn stage_result_sink_wakes_poller_after_recording_stage_completed() {
        // A1: the sink must fire the poller wake right after recording the
        // StageCompleted event so the poller re-ticks the run at the stage
        // boundary instead of after a full poll interval.
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Triage).await;
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_stage_thread(
            &thread_service,
            &workflow_run_id,
            &stage_run_id,
            GithubIssueStage::Triage,
        )
        .await;
        let (wake_sender, wake_receiver) = GithubIssueWorkflowPollerWakeReceiver::channel();
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            wake_sender,
        );

        let ack = sink
            .report_stage_result(
                executing_thread_for(&workflow_run_id, &stage_run_id),
                ReportWorkflowStageResultInput {
                    workflow_run_id: Some(workflow_run_id.as_str().to_string()),
                    stage_run_id: Some(stage_run_id.as_str().to_string()),
                    turn_run_id: Some(TurnRunId::new().to_string()),
                    stage: "triage".to_string(),
                    schema_version: "triage.v1".to_string(),
                    completion_nonce: Some("nonce-triage".to_string()),
                    result: triage_result(),
                },
            )
            .await
            .expect("stage result should be accepted");
        assert!(ack.accepted);

        // The wake was fired during `report_stage_result`. Because `Notify`
        // retains a single pending permit, a `notified()` issued after the fact
        // resolves immediately; a missing wake would make this future hang, so
        // a short timeout asserts the wake arrived.
        tokio::time::timeout(std::time::Duration::from_secs(1), wake_receiver.notified())
            .await
            .expect("poller wake should have been fired after recording StageCompleted");
    }

    #[tokio::test]
    async fn stage_result_sink_rejects_when_executing_thread_targets_other_active_stage() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        // Stage A: the stage whose thread the turn is actually executing in.
        let (workflow_run_id_a, stage_run_id_a) =
            create_active_stage(&repository, GithubIssueStage::Triage).await;
        // Stage B: a different run's active stage the model tries to complete.
        let other_issue = GithubIssueRef {
            owner: "nearai".to_string(),
            repo: "ironclaw".to_string(),
            number: 9999,
            node_id: Some("issue-node-stage-result-sink-other".to_string()),
            url: "https://github.com/nearai/ironclaw/issues/9999".to_string(),
            default_branch: "main".to_string(),
        };
        let (workflow_run_id_b, stage_run_id_b) =
            create_active_stage_with_issue(&repository, GithubIssueStage::Triage, other_issue)
                .await;

        let thread_service = Arc::new(InMemorySessionThreadService::default());
        // Only stage A's thread exists and is the executing thread.
        seed_stage_thread(
            &thread_service,
            &workflow_run_id_a,
            &stage_run_id_a,
            GithubIssueStage::Triage,
        )
        .await;
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );

        // Turn executes in stage A's thread but reports stage B's identity.
        let error = sink
            .report_stage_result(
                executing_thread_for(&workflow_run_id_a, &stage_run_id_a),
                ReportWorkflowStageResultInput {
                    workflow_run_id: Some(workflow_run_id_b.as_str().to_string()),
                    stage_run_id: Some(stage_run_id_b.as_str().to_string()),
                    turn_run_id: Some(TurnRunId::new().to_string()),
                    stage: "triage".to_string(),
                    schema_version: "triage.v1".to_string(),
                    completion_nonce: Some("nonce-triage".to_string()),
                    result: triage_result(),
                },
            )
            .await
            .expect_err("reporting another stage's result from this thread must be rejected");
        assert!(matches!(
            error,
            ironclaw_host_runtime::WorkflowStageResultSinkError::MismatchedBinding
        ));

        // Neither run advanced: no stage was accepted.
        for run_id in [workflow_run_id_a, workflow_run_id_b] {
            let events = repository
                .list_workflow_events_after(ListWorkflowEventsAfterInput {
                    workflow_run_id: run_id,
                    after_sequence: 0,
                    limit: 10,
                })
                .await
                .expect("list workflow events");
            assert!(events.is_empty());
        }
    }

    #[tokio::test]
    async fn stage_result_sink_rejects_when_executing_thread_id_absent() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Triage).await;
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_stage_thread(
            &thread_service,
            &workflow_run_id,
            &stage_run_id,
            GithubIssueStage::Triage,
        )
        .await;
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );

        // An executing thread with no thread id is unauthenticated.
        let mut executing = executing_thread_for(&workflow_run_id, &stage_run_id);
        executing.scope.thread_id = None;

        let error = sink
            .report_stage_result(
                executing,
                ReportWorkflowStageResultInput {
                    workflow_run_id: Some(workflow_run_id.as_str().to_string()),
                    stage_run_id: Some(stage_run_id.as_str().to_string()),
                    turn_run_id: Some(TurnRunId::new().to_string()),
                    stage: "triage".to_string(),
                    schema_version: "triage.v1".to_string(),
                    completion_nonce: Some("nonce-triage".to_string()),
                    result: triage_result(),
                },
            )
            .await
            .expect_err("an absent executing thread id must be rejected");
        assert!(matches!(
            error,
            ironclaw_host_runtime::WorkflowStageResultSinkError::MismatchedBinding
        ));

        let events = repository
            .list_workflow_events_after(ListWorkflowEventsAfterInput {
                workflow_run_id,
                after_sequence: 0,
                limit: 10,
            })
            .await
            .expect("list workflow events");
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn stage_result_sink_ignores_model_supplied_nonce() {
        // Both a garbage nonce and an empty nonce are accepted when the thread
        // binding matches: the nonce carries no authority — the host-derived
        // thread binding is the authority.
        for nonce in ["totally-bogus-nonce", ""] {
            let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
            let (workflow_run_id, stage_run_id) =
                create_active_stage(&repository, GithubIssueStage::Triage).await;
            let thread_service = Arc::new(InMemorySessionThreadService::default());
            seed_stage_thread(
                &thread_service,
                &workflow_run_id,
                &stage_run_id,
                GithubIssueStage::Triage,
            )
            .await;
            let sink = GithubWorkflowStageResultSink::new(
                repository.clone(),
                thread_service.clone(),
                sink_agent(),
                test_poller_wake(),
            );

            let ack = sink
                .report_stage_result(
                    executing_thread_for(&workflow_run_id, &stage_run_id),
                    ReportWorkflowStageResultInput {
                        workflow_run_id: Some(workflow_run_id.as_str().to_string()),
                        stage_run_id: Some(stage_run_id.as_str().to_string()),
                        turn_run_id: Some(TurnRunId::new().to_string()),
                        stage: "triage".to_string(),
                        schema_version: "triage.v1".to_string(),
                        completion_nonce: Some(nonce.to_string()),
                        result: triage_result(),
                    },
                )
                .await
                .expect("matching thread binding must accept regardless of the nonce");
            assert!(ack.accepted);
        }
    }

    #[tokio::test]
    async fn stage_result_sink_accepts_when_model_omits_optional_identity_fields() {
        // The input schema no longer requires workflow_run_id/stage_run_id/
        // turn_run_id/completion_nonce — the model is never told them. Supplying
        // only {stage, schema_version, result} must succeed (the host derives the
        // authoritative identity from the executing thread), and the ack must
        // carry the authoritative stage run id.
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Triage).await;
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_stage_thread(
            &thread_service,
            &workflow_run_id,
            &stage_run_id,
            GithubIssueStage::Triage,
        )
        .await;
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );

        let ack = sink
            .report_stage_result(
                executing_thread_for(&workflow_run_id, &stage_run_id),
                ReportWorkflowStageResultInput {
                    workflow_run_id: None,
                    stage_run_id: None,
                    turn_run_id: None,
                    stage: "triage".to_string(),
                    schema_version: "triage.v1".to_string(),
                    completion_nonce: None,
                    result: triage_result(),
                },
            )
            .await
            .expect("omitting the optional identity fields must be accepted");
        assert!(ack.accepted);
        assert_eq!(ack.stage_run_id, stage_run_id.as_str());

        let events = repository
            .list_workflow_events_after(ListWorkflowEventsAfterInput {
                workflow_run_id,
                after_sequence: 0,
                limit: 10,
            })
            .await
            .expect("list workflow events");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].workflow_event_type,
            GithubIssueWorkflowEventType::StageCompleted
        );
    }

    #[tokio::test]
    async fn stage_result_sink_accepts_executing_scope_from_real_turn_scope_conversion() {
        // Regression guard for the #4 scope match. Instead of a hand-authored
        // executing scope (which would mask divergence), derive it the way the
        // runtime does: build the SAME `TurnScope` the submitter writes, then run
        // the REAL `TurnScope::to_resource_scope()` conversion. If that conversion
        // ever stops reconstructing to the persisted thread scope (e.g. starts
        // setting mission_id, or maps the owner differently), read_thread's
        // exact-scope check fails and this test catches it before a live run.
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Triage).await;
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_stage_thread(
            &thread_service,
            &workflow_run_id,
            &stage_run_id,
            GithubIssueStage::Triage,
        )
        .await;
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );

        // Mirror IronClawStageTurnSubmitter::submit_accepted_message: it builds the
        // turn scope from the thread scope it wrote, and the runtime stamps the
        // executing ResourceScope via TurnScope::to_resource_scope().
        let write_scope = stage_thread_scope();
        let turn_scope = TurnScope::new_with_owner(
            write_scope.tenant_id.clone(),
            Some(write_scope.agent_id.clone()),
            write_scope.project_id.clone(),
            stage_thread_id(&workflow_run_id, &stage_run_id),
            write_scope.owner_user_id.clone(),
        );
        let executing = ExecutingStageThread {
            scope: turn_scope.to_resource_scope(),
        };

        let ack = sink
            .report_stage_result(
                executing,
                ReportWorkflowStageResultInput {
                    workflow_run_id: None,
                    stage_run_id: None,
                    turn_run_id: None,
                    stage: "triage".to_string(),
                    schema_version: "triage.v1".to_string(),
                    completion_nonce: None,
                    result: triage_result(),
                },
            )
            .await
            .expect("real TurnScope::to_resource_scope() must reconstruct to the write-side scope");
        assert!(ack.accepted);
    }

    #[tokio::test]
    async fn stage_result_sink_rejects_invalid_implementation_without_recording_event() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Implementation).await;
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_stage_thread(
            &thread_service,
            &workflow_run_id,
            &stage_run_id,
            GithubIssueStage::Implementation,
        )
        .await;
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );

        let error = sink
            .report_stage_result(
                executing_thread_for(&workflow_run_id, &stage_run_id),
                ReportWorkflowStageResultInput {
                    workflow_run_id: Some(workflow_run_id.as_str().to_string()),
                    stage_run_id: Some(stage_run_id.as_str().to_string()),
                    turn_run_id: Some(TurnRunId::new().to_string()),
                    stage: "implementation".to_string(),
                    schema_version: "implementation.v1".to_string(),
                    completion_nonce: Some("nonce-implementation".to_string()),
                    result: json!({
                        "outcome": "completed",
                        "summary": "implementation claims PR readiness without commands",
                        "evidence": [],
                        "next_actions": [],
                        "payload": {
                            "changed_files": ["src/lib.rs"],
                            "test_evidence": ["not enough"],
                            "pr_ready": true
                        }
                    }),
                },
            )
            .await
            .expect_err("missing commands_run must fail validation");

        assert!(matches!(
            error,
            ironclaw_host_runtime::WorkflowStageResultSinkError::ValidationFailed { .. }
        ));

        let events = repository
            .list_workflow_events_after(ListWorkflowEventsAfterInput {
                workflow_run_id,
                after_sequence: 0,
                limit: 10,
            })
            .await
            .expect("list workflow events");
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn stage_result_sink_events_drive_policy_to_draft_pr_once() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let github = Arc::new(FakeGithubPort::new());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );
        let policy = GithubIssueWorkflowPolicy::new(
            FakePolicyPorts {
                repository: repository.clone(),
                github: github.clone(),
                stage_turns: Arc::new(FakeStageTurnSubmitter::default()),
                project_access: Arc::new(FakeProjectAccess),
                workspace: Arc::new(FakeWorkspaceManager),
                clock: Arc::new(FakeClock::new()),
                worker_id: worker(),
            },
            "stage-result-smoke-v1",
        );

        let run = create_claimed_run(&repository).await;
        let run = set_mode(
            &repository,
            run,
            GithubIssueWorkflowMode::Implementation,
            None,
        )
        .await;
        let run = attach_workspace_session(&repository, run).await;
        let run = create_stage_run(&repository, run, GithubIssueStage::Implementation).await;
        report_stage_result(
            &sink,
            &thread_service,
            &run,
            GithubIssueStage::Implementation,
            "implementation.v1",
            implementation_result(),
        )
        .await;

        let implementation_run = current_run(&repository).await;
        let pr_synthesis = policy.tick(implementation_run).await.expect("policy tick");
        assert_eq!(
            pr_synthesis.run.workflow_state.mode,
            GithubIssueWorkflowMode::PrSynthesis
        );
        assert_eq!(policy.ports().stage_turns.requests().await.len(), 1);

        let run = current_run(&repository).await;
        report_stage_result(
            &sink,
            &thread_service,
            &run,
            GithubIssueStage::PrSynthesis,
            "pr_synthesis.v1",
            pr_synthesis_result(),
        )
        .await;

        let pr_run = current_run(&repository).await;
        let first = policy.tick(pr_run).await.expect("draft PR policy tick");
        let second = policy.tick(first.run.clone()).await.expect("replay tick");

        assert_eq!(
            first.run.workflow_state.mode,
            GithubIssueWorkflowMode::PrOpen
        );
        assert_eq!(second.processed_event_count, 0);
        assert_eq!(github.created_prs().await.len(), 1);
        assert_eq!(
            first
                .run
                .workflow_state
                .primary_pr
                .as_ref()
                .map(|pull_request| pull_request.number),
            Some(123)
        );
    }

    async fn create_active_stage(
        repository: &InMemoryGithubIssueWorkflowRepository,
        stage: GithubIssueStage,
    ) -> (
        ironclaw_github_issue_workflow::GithubIssueWorkflowRunId,
        ironclaw_github_issue_workflow::GithubIssueStageRunId,
    ) {
        create_active_stage_with_issue(repository, stage, issue()).await
    }

    async fn create_active_stage_with_issue(
        repository: &InMemoryGithubIssueWorkflowRepository,
        stage: GithubIssueStage,
        issue_ref: GithubIssueRef,
    ) -> (
        ironclaw_github_issue_workflow::GithubIssueWorkflowRunId,
        ironclaw_github_issue_workflow::GithubIssueStageRunId,
    ) {
        let run = match repository
            .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
                tenant_id: TenantId::new("tenant-stage-result-sink").unwrap(),
                creator_user_id: UserId::new("user-stage-result-sink").unwrap(),
                agent_id: Some(AgentId::new("agent-stage-result-sink").unwrap()),
                project_id: Some(ProjectId::new("project-stage-result-sink").unwrap()),
                provider_account_ref: None,
                issue_ref,
                workflow_policy_key: "github-bug-workflow".to_string(),
                workflow_policy_version: "stage-result-sink-test".to_string(),
                now: chrono::Utc::now(),
            })
            .await
            .expect("create workflow run")
        {
            ironclaw_github_issue_workflow::CreateOrGetWorkflowRunOutcome::Created { run }
            | ironclaw_github_issue_workflow::CreateOrGetWorkflowRunOutcome::Existing { run } => {
                run
            }
        };
        assert_eq!(
            run.workflow_run_key,
            GithubIssueWorkflowRunKey::for_issue(&run.issue_ref).expect("workflow run key")
        );

        match repository
            .create_stage_run(CreateStageRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                stage,
                now: chrono::Utc::now(),
            })
            .await
            .expect("create stage run")
        {
            ironclaw_github_issue_workflow::CreateStageRunOutcome::Created {
                stage_run_id, ..
            }
            | ironclaw_github_issue_workflow::CreateStageRunOutcome::ActiveStageExists {
                existing_stage_run_id: stage_run_id,
                ..
            } => (run.workflow_run_id, stage_run_id),
            ironclaw_github_issue_workflow::CreateStageRunOutcome::Terminal => {
                panic!("new run should not be terminal")
            }
        }
    }

    fn issue() -> GithubIssueRef {
        GithubIssueRef {
            owner: "nearai".to_string(),
            repo: "ironclaw".to_string(),
            number: 4242,
            node_id: Some("issue-node-stage-result-sink".to_string()),
            url: "https://github.com/nearai/ironclaw/issues/4242".to_string(),
            default_branch: "main".to_string(),
        }
    }

    async fn create_claimed_run(
        repository: &InMemoryGithubIssueWorkflowRepository,
    ) -> GithubIssueWorkflowRun {
        let run = match repository
            .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
                tenant_id: TenantId::new("tenant-stage-result-sink").unwrap(),
                creator_user_id: UserId::new("user-stage-result-sink").unwrap(),
                agent_id: Some(AgentId::new("agent-stage-result-sink").unwrap()),
                project_id: Some(ProjectId::new("project-stage-result-sink").unwrap()),
                provider_account_ref: Some(provider_account_ref()),
                issue_ref: issue(),
                workflow_policy_key: "github-bug-workflow".to_string(),
                workflow_policy_version: "stage-result-sink-test".to_string(),
                now: chrono::Utc::now(),
            })
            .await
            .expect("create workflow run")
        {
            CreateOrGetWorkflowRunOutcome::Created { run }
            | CreateOrGetWorkflowRunOutcome::Existing { run } => run,
        };

        repository
            .claim_runnable_workflow_runs(ClaimRunnableWorkflowRunsInput {
                tenant_id: run.tenant_id.clone(),
                worker_id: worker(),
                now: chrono::Utc::now(),
                lease_expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
                limit: 1,
            })
            .await
            .expect("claim workflow run")
            .pop()
            .unwrap_or(run)
    }

    async fn current_run(
        repository: &InMemoryGithubIssueWorkflowRepository,
    ) -> GithubIssueWorkflowRun {
        match repository
            .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
                tenant_id: TenantId::new("tenant-stage-result-sink").unwrap(),
                creator_user_id: UserId::new("user-stage-result-sink").unwrap(),
                agent_id: Some(AgentId::new("agent-stage-result-sink").unwrap()),
                project_id: Some(ProjectId::new("project-stage-result-sink").unwrap()),
                provider_account_ref: Some(provider_account_ref()),
                issue_ref: issue(),
                workflow_policy_key: "github-bug-workflow".to_string(),
                workflow_policy_version: "stage-result-sink-test".to_string(),
                now: chrono::Utc::now(),
            })
            .await
            .expect("get current workflow run")
        {
            CreateOrGetWorkflowRunOutcome::Created { run }
            | CreateOrGetWorkflowRunOutcome::Existing { run } => run,
        }
    }

    async fn set_mode(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: GithubIssueWorkflowRun,
        mode: GithubIssueWorkflowMode,
        primary_pr: Option<GithubPullRequestRef>,
    ) -> GithubIssueWorkflowRun {
        match repository
            .advance_event_cursor_and_transition(AdvanceWorkflowRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                worker_id: worker(),
                expected_workflow_run_version: run.workflow_run_version,
                expected_event_cursor: run.event_cursor,
                next_event_cursor: run.event_cursor,
                transition: WorkflowRunTransition {
                    mode: Some(mode),
                    primary_pr,
                    clear_active_block: true,
                    ..WorkflowRunTransition::default()
                },
                now: chrono::Utc::now(),
            })
            .await
            .expect("set workflow mode")
        {
            TransitionOutcome::Applied { run } => run,
            other => panic!("mode transition should apply: {other:?}"),
        }
    }

    async fn attach_workspace_session(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: GithubIssueWorkflowRun,
    ) -> GithubIssueWorkflowRun {
        let workspace_session_id =
            GithubIssueWorkspaceSessionId::from_trusted("workspace-session-smoke".to_string())
                .unwrap();
        let session = GithubIssueWorkspaceSession {
            workspace_session_id: workspace_session_id.clone(),
            workflow_run_id: run.workflow_run_id.clone(),
            repository: GithubRepositorySelector {
                owner: run.issue_ref.owner.clone(),
                repo: run.issue_ref.repo.clone(),
            },
            base_branch: run.issue_ref.default_branch.clone(),
            base_sha: None,
            working_branch: "ironclaw/fix-4242".to_string(),
            current_head_sha: Some("head-sha-4242".to_string()),
            workspace_ref: WorkflowWorkspaceRef {
                thread_id: None,
                workspace_session_id: Some(workspace_session_id),
                turn_run_id: None,
            },
            mount_ref: WorkflowWorkspaceMountRef {
                mount_id: "workspace-mount-smoke".to_string(),
                alias: "/workspace".to_string(),
            },
            created_at: chrono::Utc::now(),
        };
        match repository
            .advance_event_cursor_and_transition(AdvanceWorkflowRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                worker_id: worker(),
                expected_workflow_run_version: run.workflow_run_version,
                expected_event_cursor: run.event_cursor,
                next_event_cursor: run.event_cursor,
                transition: WorkflowRunTransition {
                    workspace_session: Some(session),
                    ..WorkflowRunTransition::default()
                },
                now: chrono::Utc::now(),
            })
            .await
            .expect("attach workspace session")
        {
            TransitionOutcome::Applied { run } => run,
            other => panic!("workspace session transition should apply: {other:?}"),
        }
    }

    async fn create_stage_run(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: GithubIssueWorkflowRun,
        stage: GithubIssueStage,
    ) -> GithubIssueWorkflowRun {
        match repository
            .create_stage_run(CreateStageRunInput {
                workflow_run_id: run.workflow_run_id,
                stage,
                now: chrono::Utc::now(),
            })
            .await
            .expect("create stage run")
        {
            ironclaw_github_issue_workflow::CreateStageRunOutcome::Created { run, .. }
            | ironclaw_github_issue_workflow::CreateStageRunOutcome::ActiveStageExists {
                run,
                ..
            } => run,
            ironclaw_github_issue_workflow::CreateStageRunOutcome::Terminal => {
                panic!("workflow run should not be terminal")
            }
        }
    }

    async fn report_stage_result(
        sink: &GithubWorkflowStageResultSink,
        thread_service: &InMemorySessionThreadService,
        run: &GithubIssueWorkflowRun,
        stage: GithubIssueStage,
        schema_version: &str,
        result: serde_json::Value,
    ) {
        let stage_run_id = run
            .active_stage_run_id
            .as_ref()
            .expect("active stage")
            .clone();
        seed_stage_thread(
            thread_service,
            &run.workflow_run_id,
            &stage_run_id,
            stage.clone(),
        )
        .await;
        let ack = sink
            .report_stage_result(
                executing_thread_for(&run.workflow_run_id, &stage_run_id),
                ReportWorkflowStageResultInput {
                    workflow_run_id: Some(run.workflow_run_id.as_str().to_string()),
                    stage_run_id: Some(stage_run_id.as_str().to_string()),
                    turn_run_id: Some(TurnRunId::new().to_string()),
                    stage: stage_name(&stage).to_string(),
                    schema_version: schema_version.to_string(),
                    completion_nonce: Some(format!("nonce-{schema_version}")),
                    result,
                },
            )
            .await
            .expect("stage result should be accepted");
        assert!(ack.accepted);
    }

    fn stage_name(stage: &GithubIssueStage) -> &'static str {
        match stage {
            GithubIssueStage::Triage => "triage",
            GithubIssueStage::Planning => "planning",
            GithubIssueStage::Implementation => "implementation",
            GithubIssueStage::PrSynthesis => "pr_synthesis",
            GithubIssueStage::CiRepair => "ci_repair",
            GithubIssueStage::ReviewResponse => "review_response",
        }
    }

    fn triage_result() -> serde_json::Value {
        json!({
            "outcome": "completed",
            "summary": "triage completed",
            "evidence": [],
            "next_actions": [],
            "payload": {
                "is_reproducible": true,
                "suspected_area": "composition sink",
                "risk": "medium",
                "recommended_next_stage": "planning"
            }
        })
    }

    fn implementation_result() -> serde_json::Value {
        json!({
            "outcome": "completed",
            "summary": "implementation completed",
            "evidence": [],
            "next_actions": [],
            "payload": {
                "changed_files": ["src/lib.rs"],
                "commands_run": ["cargo test"],
                "test_evidence": ["tests passed"],
                "pr_ready": true
            }
        })
    }

    fn pr_synthesis_result() -> serde_json::Value {
        json!({
            "outcome": "completed",
            "summary": "draft PR ready",
            "evidence": [],
            "next_actions": [],
            "payload": {
                "title": "Fix issue 4242",
                "body": "This fixes issue 4242.",
                "branch_name": "ironclaw/fix-4242",
                "base_branch": "main",
                "head_sha": "head-sha-4242"
            }
        })
    }

    fn provider_account_ref() -> GithubProviderAccountRef {
        GithubProviderAccountRef {
            provider: "github".to_string(),
            account_id: "github-stage-result-sink".to_string(),
        }
    }

    fn worker() -> WorkflowWorkerId {
        WorkflowWorkerId::from_trusted("worker-stage-result-sink".to_string()).unwrap()
    }

    struct FakeClock {
        now: StdMutex<chrono::DateTime<chrono::Utc>>,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                now: StdMutex::new(chrono::Utc::now()),
            }
        }
    }

    impl WorkflowClock for FakeClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            *self.now.lock().expect("clock lock")
        }
    }

    #[derive(Debug)]
    struct FakeGithubPort {
        created_prs: Mutex<Vec<CreateDraftPullRequestInput>>,
        create_pr_results: Mutex<VecDeque<Result<GithubPullRequestRef, GithubIssueWorkflowError>>>,
    }

    impl FakeGithubPort {
        fn new() -> Self {
            Self {
                created_prs: Mutex::new(Vec::new()),
                create_pr_results: Mutex::new(VecDeque::from([Ok(GithubPullRequestRef {
                    owner: "nearai".to_string(),
                    repo: "ironclaw".to_string(),
                    number: 123,
                    node_id: Some("pr-node-123".to_string()),
                    url: "https://github.com/nearai/ironclaw/pull/123".to_string(),
                    head_branch: "ironclaw/fix-4242".to_string(),
                    head_sha: Some("head-sha-4242".to_string()),
                })])),
            }
        }

        async fn created_prs(&self) -> Vec<CreateDraftPullRequestInput> {
            self.created_prs.lock().await.clone()
        }
    }

    #[async_trait]
    impl ironclaw_github_issue_workflow::GithubIssueWorkflowPort for FakeGithubPort {
        async fn get_authenticated_workflow_actor(
            &self,
            _input: GetAuthenticatedWorkflowActorInput,
        ) -> Result<GithubActorSnapshot, GithubIssueWorkflowError> {
            Ok(GithubActorSnapshot {
                login: "ironclaw-bot".to_string(),
                node_id: Some("actor-node-stage-result-sink".to_string()),
            })
        }

        async fn list_issue_comments(
            &self,
            _input: ListIssueCommentsInput,
        ) -> Result<Vec<GithubIssueCommentSnapshot>, GithubIssueWorkflowError> {
            Ok(Vec::new())
        }

        async fn create_issue_comment(
            &self,
            _input: CreateIssueCommentInput,
        ) -> Result<GithubCommentRef, GithubIssueWorkflowError> {
            Ok(GithubCommentRef {
                node_id: Some("comment-node-stage-result-sink".to_string()),
                url: "https://github.com/nearai/ironclaw/issues/4242#issuecomment-1".to_string(),
            })
        }

        async fn list_pull_requests(
            &self,
            _input: ListPullRequestsInput,
        ) -> Result<Vec<GithubPullRequestSnapshot>, GithubIssueWorkflowError> {
            Ok(Vec::new())
        }

        async fn create_draft_pull_request(
            &self,
            input: CreateDraftPullRequestInput,
        ) -> Result<GithubPullRequestRef, GithubIssueWorkflowError> {
            self.created_prs.lock().await.push(input);
            self.create_pr_results
                .lock()
                .await
                .pop_front()
                .unwrap_or_else(|| {
                    Err(GithubIssueWorkflowError::ProviderRead {
                        reason: "unexpected draft PR retry".to_string(),
                    })
                })
        }
    }

    #[derive(Debug, Default)]
    struct FakeProjectAccess;

    #[async_trait]
    impl WorkflowProjectAccess for FakeProjectAccess {
        async fn assert_workflow_project_access(
            &self,
            _request: WorkflowProjectAccessRequest,
        ) -> Result<(), GithubIssueWorkflowError> {
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct FakeWorkspaceManager;

    #[async_trait]
    impl WorkflowWorkspaceManager for FakeWorkspaceManager {
        async fn prepare_workspace(
            &self,
            request: PrepareWorkflowWorkspaceRequest,
        ) -> Result<PrepareWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
            let workspace_session_id =
                GithubIssueWorkspaceSessionId::from_trusted("workspace-session-smoke".to_string())
                    .unwrap();
            Ok(PrepareWorkflowWorkspaceOutcome {
                session: GithubIssueWorkspaceSession {
                    workspace_session_id: workspace_session_id.clone(),
                    workflow_run_id: request.workflow_run_id,
                    repository: GithubRepositorySelector {
                        owner: request.issue.owner,
                        repo: request.issue.repo,
                    },
                    base_branch: request.base_branch,
                    base_sha: None,
                    working_branch: "ironclaw/fix-4242".to_string(),
                    current_head_sha: Some("head-sha-4242".to_string()),
                    workspace_ref: WorkflowWorkspaceRef {
                        thread_id: Some(ThreadId::new("workspace-thread-smoke").unwrap()),
                        workspace_session_id: Some(workspace_session_id),
                        turn_run_id: Some(TurnRunId::new()),
                    },
                    mount_ref: WorkflowWorkspaceMountRef {
                        mount_id: "workspace-mount-smoke".to_string(),
                        alias: "/workspace".to_string(),
                    },
                    created_at: request.requested_at,
                },
            })
        }

        async fn publish_workspace(
            &self,
            request: PublishWorkflowWorkspaceRequest,
        ) -> Result<PublishWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
            Ok(PublishWorkflowWorkspaceOutcome {
                working_branch: "ironclaw/fix-4242".to_string(),
                base_branch: request.base_branch,
                head_sha: "head-sha-4242".to_string(),
                has_changes: true,
            })
        }
    }

    #[derive(Debug, Default)]
    struct FakeStageTurnSubmitter {
        requests: Mutex<Vec<SubmitStageTurnRequest>>,
    }

    impl FakeStageTurnSubmitter {
        async fn requests(&self) -> Vec<SubmitStageTurnRequest> {
            self.requests.lock().await.clone()
        }
    }

    #[async_trait]
    impl StageTurnSubmitter for FakeStageTurnSubmitter {
        async fn submit_stage_turn(
            &self,
            request: SubmitStageTurnRequest,
        ) -> Result<SubmitStageTurnOutcome, GithubIssueWorkflowError> {
            let request_count = {
                let mut requests = self.requests.lock().await;
                requests.push(request);
                requests.len()
            };
            Ok(SubmitStageTurnOutcome::Submitted {
                thread_id: ThreadId::new(format!("thread-stage-result-sink-{request_count}"))
                    .unwrap(),
                turn_run_id: TurnRunId::new(),
            })
        }
    }

    struct FakePolicyPorts {
        repository: Arc<InMemoryGithubIssueWorkflowRepository>,
        github: Arc<FakeGithubPort>,
        stage_turns: Arc<FakeStageTurnSubmitter>,
        project_access: Arc<FakeProjectAccess>,
        workspace: Arc<FakeWorkspaceManager>,
        clock: Arc<FakeClock>,
        worker_id: WorkflowWorkerId,
    }

    impl GithubIssueWorkflowPolicyPorts for FakePolicyPorts {
        type Clock = FakeClock;
        type GithubPort = FakeGithubPort;
        type ProjectAccess = FakeProjectAccess;
        type Repository = InMemoryGithubIssueWorkflowRepository;
        type StageTurnSubmitter = FakeStageTurnSubmitter;
        type WorkspaceManager = FakeWorkspaceManager;

        fn clock(&self) -> Arc<Self::Clock> {
            self.clock.clone()
        }

        fn github_port(&self) -> Arc<Self::GithubPort> {
            self.github.clone()
        }

        fn project_access(&self) -> Arc<Self::ProjectAccess> {
            self.project_access.clone()
        }

        fn repository(&self) -> Arc<Self::Repository> {
            self.repository.clone()
        }

        fn stage_turn_submitter(&self) -> Arc<Self::StageTurnSubmitter> {
            self.stage_turns.clone()
        }

        fn workspace_manager(&self) -> Arc<Self::WorkspaceManager> {
            self.workspace.clone()
        }

        fn worker_id(&self) -> WorkflowWorkerId {
            self.worker_id.clone()
        }
    }
}

pub(crate) struct GithubIssueWorkflowRuntimeHandle {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

impl GithubIssueWorkflowRuntimeHandle {
    pub(crate) async fn shutdown(self, timeout: Duration) {
        self.cancel.cancel();
        let mut handle = self.handle;
        match tokio::time::timeout(timeout, &mut handle).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                // Background task: debug! only (CLAUDE.md REPL/TUI rule).
                tracing::debug!(?error, "GitHub issue workflow poller task join failed");
            }
            Err(_) => {
                tracing::debug!(
                    ?timeout,
                    "GitHub issue workflow poller did not stop before shutdown timeout; aborting"
                );
                handle.abort();
                if let Err(error) = handle.await
                    && error.is_panic()
                {
                    tracing::debug!(?error, "aborted GitHub issue workflow poller task panicked");
                }
            }
        }
    }
}

pub(crate) struct GithubIssueWorkflowRuntimeDeps {
    pub(crate) repository: Arc<dyn GithubIssueWorkflowRepository>,
    pub(crate) stage_result_sink_slot: Arc<WorkflowStageResultSinkSlot>,
    pub(crate) host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime>,
    pub(crate) config_source: Arc<dyn GithubIssueWorkflowConfigSource>,
    pub(crate) project_access: Arc<dyn WorkflowProjectAccess>,
    pub(crate) workspace_manager: Arc<dyn WorkflowWorkspaceManager>,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) turn_coordinator: Arc<dyn TurnCoordinator>,
    pub(crate) tenant_id: TenantId,
    pub(crate) actor_user_id: UserId,
    pub(crate) default_agent_id: AgentId,
    pub(crate) default_project_id: Option<ProjectId>,
}

pub(crate) fn spawn_github_issue_workflow(
    settings: crate::runtime_input::GithubIssueWorkflowSettings,
    deps: GithubIssueWorkflowRuntimeDeps,
) -> Result<Option<GithubIssueWorkflowRuntimeHandle>, GithubIssueWorkflowError> {
    if !settings.enabled {
        return Ok(None);
    }
    validate_github_issue_workflow_settings(&settings)?;
    let GithubIssueWorkflowRuntimeDeps {
        repository,
        stage_result_sink_slot,
        host_runtime,
        config_source,
        project_access,
        workspace_manager,
        thread_service,
        turn_coordinator,
        tenant_id,
        actor_user_id,
        default_agent_id,
        default_project_id,
    } = deps;
    // Wake channel: the stage-result sink fires `wake_sender` at each stage
    // boundary; the poller loop selects on `wake_receiver` so it re-ticks the
    // affected run immediately instead of after a full poll interval.
    let (wake_sender, wake_receiver) = GithubIssueWorkflowPollerWakeReceiver::channel();
    let sink: Arc<dyn WorkflowStageResultSink> = Arc::new(GithubWorkflowStageResultSink::new(
        Arc::clone(&repository),
        Arc::clone(&thread_service),
        default_agent_id.clone(),
        wake_sender,
    ));
    stage_result_sink_slot
        .set(sink)
        .map_err(|_| GithubIssueWorkflowError::InvalidConfig {
            reason: "workflow stage result sink slot was already initialized".to_string(),
        })?;

    let dispatcher = Arc::new(HostRuntimeGithubIssueWorkflowCapabilityDispatcher::new(
        host_runtime,
        workflow_execution_context(
            tenant_id,
            actor_user_id.clone(),
            default_agent_id.clone(),
            default_project_id,
        )?,
        workflow_trust_decision(),
    ));
    let github_port = Arc::new(IronClawGithubIssueWorkflowPort::new(dispatcher));
    let stage_turn_submitter = Arc::new(IronClawStageTurnSubmitter::new(
        thread_service,
        turn_coordinator,
        actor_user_id,
        default_agent_id,
    ));
    let poller = GithubIssueWorkflowPoller::new(
        IronClawGithubIssueWorkflowPollerPorts {
            clock: Arc::new(SystemWorkflowClock),
            config_source,
            github_port,
            project_access,
            repository,
            stage_turn_submitter,
            workspace_manager,
            worker_id: WorkflowWorkerId::new(),
        },
        GithubIssueWorkflowPollerConfig {
            enabled: true,
            poll_interval: settings.poll_interval,
            max_repos_per_tick: settings.max_repos_per_tick,
            max_issues_per_repo_per_tick: settings.max_issues_per_repo_per_tick,
            max_runnable_runs_per_tick: settings.max_runnable_runs_per_tick,
            lease_duration: settings.lease_duration,
            stage_stale_after: settings.stage_stale_after,
        },
        "github-bug-workflow-v1",
    );
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let poll_interval = settings.poll_interval;
    // ACTIVATION PRECONDITION: the poller dispatches the `github.*` capabilities
    // (search_issues/get_issue/comment_issue/create_pull_request/…) through the
    // host runtime, which only resolves them once the operator has INSTALLED AND
    // ACTIVATED the bundled `github` extension (via the WebUI extensions surface
    // or the `builtin.extension_activate` tool) and configured the ProductAuth
    // github account referenced by the provider account id. Activation also
    // grants the extension `user_trusted` trust, so no static trust-policy entry
    // is needed. We intentionally do NOT preflight-and-fail here: activation
    // happens post-boot through the same running server, so a hard failure would
    // deadlock startup. Until github is activated, the poller simply idles —
    // every tick's first provider call fails with `unknown_capability`, surfaced
    // (per-tick) at `debug!` by run_github_issue_workflow_poller — and recovers
    // automatically once the extension is activated. See the live-run runbook.
    let handle = tokio::spawn(async move {
        run_github_issue_workflow_poller(poller, poll_interval, wake_receiver, task_cancel).await;
    });
    Ok(Some(GithubIssueWorkflowRuntimeHandle { cancel, handle }))
}

fn validate_github_issue_workflow_settings(
    settings: &crate::runtime_input::GithubIssueWorkflowSettings,
) -> Result<(), GithubIssueWorkflowError> {
    if settings.poll_interval.is_zero() {
        return Err(GithubIssueWorkflowError::InvalidConfig {
            reason: "poll_interval must be greater than zero".to_string(),
        });
    }
    if settings.lease_duration.is_zero() {
        return Err(GithubIssueWorkflowError::InvalidConfig {
            reason: "lease_duration must be greater than zero".to_string(),
        });
    }
    if settings.max_repos_per_tick == 0 {
        return Err(GithubIssueWorkflowError::InvalidConfig {
            reason: "max_repos_per_tick must be greater than zero".to_string(),
        });
    }
    if settings.max_issues_per_repo_per_tick == 0 {
        return Err(GithubIssueWorkflowError::InvalidConfig {
            reason: "max_issues_per_repo_per_tick must be greater than zero".to_string(),
        });
    }
    if settings.max_runnable_runs_per_tick == 0 {
        return Err(GithubIssueWorkflowError::InvalidConfig {
            reason: "max_runnable_runs_per_tick must be greater than zero".to_string(),
        });
    }
    Ok(())
}

async fn run_github_issue_workflow_poller<P>(
    poller: GithubIssueWorkflowPoller<P>,
    poll_interval: Duration,
    wake: GithubIssueWorkflowPollerWakeReceiver,
    cancel: CancellationToken,
) where
    P: GithubIssueWorkflowPollerPorts + 'static,
{
    loop {
        match poller.tick_once().await {
            Ok(outcome) => {
                tracing::debug!(
                    configs_loaded = outcome.configs_loaded,
                    repositories_scanned = outcome.repositories_scanned,
                    issues_seen = outcome.issues_seen,
                    runnable_runs_claimed = outcome.runnable_runs_claimed,
                    stale_stages_failed = outcome.stale_stages_failed,
                    blocked_configs = outcome.blocked_configs.len(),
                    blocked_runs = outcome.blocked_runs.len(),
                    "GitHub issue workflow poller tick completed"
                );
            }
            Err(error) => {
                // Background task: must use debug!/trace! only — info!/warn! corrupt
                // the REPL/TUI (CLAUDE.md). Operators tail
                // `ironclaw_github_issue_workflow=debug` for live diagnosis.
                tracing::debug!(?error, "GitHub issue workflow poller tick failed");
            }
        }
        // Wake-driven with an interval safety net: a stage-result completion
        // fires the wake so the next tick re-claims the affected run at the
        // stage boundary immediately; the interval still bounds latency for
        // provider-side changes (new issues, PR updates) that the sink never
        // sees, and recovers any dropped/coalesced wake.
        if !wait_for_wake_or_interval(poll_interval, &wake, &cancel).await {
            return;
        }
    }
}

/// Wait until either a wake signal arrives or the fallback interval elapses.
/// Returns `false` (stop the loop) on cancellation, `true` to tick again.
async fn wait_for_wake_or_interval(
    delay: Duration,
    wake: &GithubIssueWorkflowPollerWakeReceiver,
    cancel: &CancellationToken,
) -> bool {
    tokio::select! {
        _ = cancel.cancelled() => false,
        _ = wake.notified() => true,
        _ = tokio::time::sleep(delay) => true,
    }
}

fn workflow_execution_context(
    tenant_id: TenantId,
    owner_user_id: UserId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
) -> Result<ExecutionContext, GithubIssueWorkflowError> {
    let invocation_id = InvocationId::new();
    let extension_id = ExtensionId::new(WORKFLOW_ADAPTER_ID).map_err(workflow_invalid_config)?;
    let resource_scope = ResourceScope {
        tenant_id: tenant_id.clone(),
        user_id: owner_user_id.clone(),
        agent_id: Some(agent_id.clone()),
        project_id: project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    let context = ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id,
        user_id: owner_user_id,
        agent_id: Some(agent_id),
        project_id,
        mission_id: None,
        thread_id: None,
        extension_id: extension_id.clone(),
        runtime: RuntimeKind::FirstParty,
        trust: TrustClass::FirstParty,
        grants: workflow_capability_grants(&extension_id)?,
        mounts: MountView::default(),
        resource_scope,
    };
    context.validate().map_err(workflow_invalid_config)?;
    Ok(context)
}

fn workflow_trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: workflow_authority_effects(),
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

fn workflow_capability_grants(
    grantee: &ExtensionId,
) -> Result<CapabilitySet, GithubIssueWorkflowError> {
    let grants = WORKFLOW_GITHUB_CAPABILITY_IDS
        .iter()
        .map(|capability_id| {
            Ok(CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: CapabilityId::new(*capability_id).map_err(workflow_invalid_config)?,
                grantee: Principal::Extension(grantee.clone()),
                issued_by: ironclaw_approvals::persistent_approval_grant_issuer(),
                constraints: GrantConstraints {
                    allowed_effects: workflow_capability_effects(capability_id),
                    mounts: MountView::default(),
                    network: workflow_github_network_policy(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            })
        })
        .collect::<Result<Vec<_>, GithubIssueWorkflowError>>()?;
    Ok(CapabilitySet { grants })
}

fn workflow_github_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.github.com".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: None,
    }
}

fn workflow_capability_effects(capability_id: &str) -> Vec<EffectKind> {
    match capability_id {
        GITHUB_COMMENT_ISSUE_CAPABILITY_ID | GITHUB_CREATE_PULL_REQUEST_CAPABILITY_ID => vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::UseSecret,
            EffectKind::ExternalWrite,
        ],
        _ => vec![EffectKind::Network, EffectKind::UseSecret],
    }
}

fn workflow_authority_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::Network,
        EffectKind::UseSecret,
        EffectKind::ExternalWrite,
    ]
}

fn workflow_invalid_config(error: impl std::fmt::Display) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::InvalidConfig {
        reason: error.to_string(),
    }
}

struct IronClawGithubIssueWorkflowPollerPorts {
    clock: Arc<dyn WorkflowClock>,
    config_source: Arc<dyn GithubIssueWorkflowConfigSource>,
    github_port: Arc<dyn GithubIssueWorkflowPort>,
    project_access: Arc<dyn WorkflowProjectAccess>,
    repository: Arc<dyn GithubIssueWorkflowRepository>,
    stage_turn_submitter: Arc<dyn StageTurnSubmitter>,
    workspace_manager: Arc<dyn WorkflowWorkspaceManager>,
    worker_id: WorkflowWorkerId,
}

impl GithubIssueWorkflowPollerPorts for IronClawGithubIssueWorkflowPollerPorts {
    type Clock = dyn WorkflowClock;
    type ConfigSource = dyn GithubIssueWorkflowConfigSource;
    type GithubPort = dyn GithubIssueWorkflowPort;
    type ProjectAccess = dyn WorkflowProjectAccess;
    type Repository = dyn GithubIssueWorkflowRepository;
    type StageTurnSubmitter = dyn StageTurnSubmitter;
    type WorkspaceManager = dyn WorkflowWorkspaceManager;

    fn clock(&self) -> Arc<Self::Clock> {
        Arc::clone(&self.clock)
    }

    fn config_source(&self) -> Arc<Self::ConfigSource> {
        Arc::clone(&self.config_source)
    }

    fn github_port(&self) -> Arc<Self::GithubPort> {
        Arc::clone(&self.github_port)
    }

    fn project_access(&self) -> Arc<Self::ProjectAccess> {
        Arc::clone(&self.project_access)
    }

    fn repository(&self) -> Arc<Self::Repository> {
        Arc::clone(&self.repository)
    }

    fn stage_turn_submitter(&self) -> Arc<Self::StageTurnSubmitter> {
        Arc::clone(&self.stage_turn_submitter)
    }

    fn workspace_manager(&self) -> Arc<Self::WorkspaceManager> {
        Arc::clone(&self.workspace_manager)
    }

    fn worker_id(&self) -> WorkflowWorkerId {
        self.worker_id.clone()
    }
}

pub(crate) fn test_only_provider_account_ref() -> GithubProviderAccountRef {
    GithubProviderAccountRef {
        provider: "github".to_string(),
        account_id: "github-issue-workflow-test".to_string(),
    }
}

pub(crate) fn test_only_unconfigured_project_access() -> Arc<dyn WorkflowProjectAccess> {
    Arc::new(UnconfiguredWorkflowProjectAccess)
}

pub(crate) fn test_only_empty_config_source() -> Arc<dyn GithubIssueWorkflowConfigSource> {
    Arc::new(EmptyGithubIssueWorkflowConfigSource)
}

pub(crate) fn test_only_unconfigured_workspace_manager() -> Arc<dyn WorkflowWorkspaceManager> {
    Arc::new(UnconfiguredWorkflowWorkspaceManager)
}

struct SystemWorkflowClock;

impl WorkflowClock for SystemWorkflowClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[cfg(test)]
mod project_metadata_github_issue_workflow_config_source_tests {
    use super::{
        IronClawGithubIssueWorkflowPollerPorts, ProjectMetadataGithubIssueWorkflowConfigSource,
        ProjectServiceWorkflowProjectAccess, RuntimeWorkflowWorkspaceManager,
        WorkflowGitRemoteConfig, git_branch_component, test_only_unconfigured_workspace_manager,
        workflow_stage_workspace_mount_view_from_thread_metadata, workflow_workspace_host_path,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_github_issue_workflow::{
        CreateIssueCommentInput, GetAuthenticatedWorkflowActorInput, GithubActorSnapshot,
        GithubCommentRef, GithubIssueCommentSnapshot, GithubIssueRef, GithubIssueSearchHit,
        GithubIssueWorkflowConfigSource, GithubIssueWorkflowError, GithubIssueWorkflowPoller,
        GithubIssueWorkflowPollerBlockKind, GithubIssueWorkflowPollerConfig,
        GithubIssueWorkflowPort, GithubProviderAccountRef, InMemoryGithubIssueWorkflowRepository,
        ListIssueCommentsInput, PrepareWorkflowWorkspaceRequest, PublishWorkflowWorkspaceRequest,
        SearchGithubIssuesInput, StageTurnSubmitter, SubmitStageTurnOutcome,
        SubmitStageTurnRequest, VerifyWorkflowWorkspaceRequest, WorkflowClock,
        WorkflowConfigAccessRequest, WorkflowProjectAccess, WorkflowProjectAccessRequest,
        WorkflowVerificationCommand, WorkflowWorkerId, WorkflowWorkspaceManager,
    };
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
    use ironclaw_product_workflow::{
        ProjectCaller, ProjectService, ProjectServiceError, RebornAddMemberRequest,
        RebornCreateProjectRequest, RebornDeleteProjectRequest, RebornGetProjectRequest,
        RebornListMembersRequest, RebornListMembersResponse, RebornListProjectsRequest,
        RebornListProjectsResponse, RebornProjectInfo, RebornProjectMemberInfo,
        RebornProjectResponse, RebornProjectRole, RebornProjectState, RebornRemoveMemberRequest,
        RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
    };
    use serde_json::{Value as JsonValue, json};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[tokio::test]
    async fn project_metadata_config_source_builds_enabled_workflow_config() {
        let metadata = json!({
            "github_issue_workflow": {
                "enabled": true,
                "repositories": [
                    { "owner": "near", "repo": "ironclaw" }
                ],
                "labels": ["bug", "regression"],
                "allowed_author_logins": ["core-dev", "repo-maintainer"],
                "max_active_runs_per_repo": 3,
                "default_run_profile": "github-bug-workflow-v1"
            }
        });
        let project_service = Arc::new(FakeProjectService::new(metadata));
        let source = source_with_service(project_service.clone());

        let configs = source
            .list_enabled_workflow_configs()
            .await
            .expect("config loads");

        assert_eq!(configs.len(), 1);
        let config = &configs[0];
        assert_eq!(config.tenant_id.as_str(), "workflow-tenant");
        assert_eq!(config.owner_user_id.as_str(), "workflow-owner");
        assert_eq!(config.project_id.as_str(), "workflow-project");
        assert_eq!(
            config.provider_account_ref.account_id,
            "runtime-github-account"
        );
        assert_eq!(config.repositories[0].owner, "near");
        assert_eq!(config.repositories[0].repo, "ironclaw");
        assert_eq!(config.candidate_selector.labels, ["bug", "regression"]);
        assert_eq!(
            config.candidate_selector.allowed_author_logins,
            ["core-dev", "repo-maintainer"]
        );
        assert_eq!(config.max_active_runs_per_repo, 3);
        assert_eq!(config.default_run_profile, "github-bug-workflow-v1");

        let captured = project_service.captured_get_project();
        assert_eq!(captured.0.tenant_id.as_str(), "workflow-tenant");
        assert_eq!(captured.0.user_id.as_str(), "workflow-owner");
        assert_eq!(captured.1.project_id, "workflow-project");
    }

    #[tokio::test]
    async fn project_metadata_config_source_ignores_missing_or_disabled_section() {
        let missing = source_with_service(Arc::new(FakeProjectService::new(json!({
            "other": true
        }))));
        assert!(
            missing
                .list_enabled_workflow_configs()
                .await
                .expect("missing metadata is allowed")
                .is_empty()
        );

        let disabled = source_with_service(Arc::new(FakeProjectService::new(json!({
            "github_issue_workflow": {
                "enabled": false
            }
        }))));
        assert!(
            disabled
                .list_enabled_workflow_configs()
                .await
                .expect("disabled metadata is allowed")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn project_metadata_config_source_rejects_untrusted_fields() {
        let source = source_with_service(Arc::new(FakeProjectService::new(json!({
            "github_issue_workflow": {
                "enabled": true,
                "repositories": [
                    { "owner": "near", "repo": "ironclaw" }
                ],
                "provider_account_ref": {
                    "provider": "github",
                    "account_id": "metadata-must-not-select-this"
                }
            }
        }))));

        let error = source
            .list_enabled_workflow_configs()
            .await
            .expect_err("untrusted provider account field must fail closed");

        assert!(
            matches!(error, GithubIssueWorkflowError::InvalidConfig { ref reason } if reason.contains("provider_account_ref")),
            "unexpected error: {error:?}"
        );
    }

    #[tokio::test]
    async fn project_service_project_access_uses_trusted_project_service() {
        let project_service = Arc::new(FakeProjectService::new(json!({
            "github_issue_workflow": {
                "enabled": true,
                "repositories": [
                    { "owner": "nearai", "repo": "ironclaw" }
                ]
            }
        })));
        let access = ProjectServiceWorkflowProjectAccess {
            project_service: project_service.clone(),
            configured_provider_account_ref: GithubProviderAccountRef {
                provider: "github".to_string(),
                account_id: "runtime-github-account".to_string(),
            },
        };

        access
            .assert_workflow_config_access(WorkflowConfigAccessRequest {
                tenant_id: TenantId::new("workflow-tenant").expect("tenant"),
                creator_user_id: UserId::new("workflow-owner").expect("user"),
                project_id: ProjectId::new("workflow-project").expect("project"),
                repositories: vec![
                    ironclaw_github_issue_workflow::GithubRepositorySelector::new(
                        "nearai", "ironclaw",
                    )
                    .expect("selector"),
                ],
                provider_account_ref: GithubProviderAccountRef {
                    provider: "github".to_string(),
                    account_id: "runtime-github-account".to_string(),
                },
            })
            .await
            .expect("config access allowed");

        access
            .assert_workflow_project_access(WorkflowProjectAccessRequest {
                tenant_id: TenantId::new("workflow-tenant").expect("tenant"),
                creator_user_id: UserId::new("workflow-owner").expect("user"),
                agent_id: Some(AgentId::new("workflow-agent").expect("agent")),
                project_id: Some(ProjectId::new("workflow-project").expect("project")),
                workflow_run_id:
                    ironclaw_github_issue_workflow::GithubIssueWorkflowRunId::from_trusted(
                        "workflow-run-project-access".to_string(),
                    )
                    .expect("workflow run"),
                issue: GithubIssueRef {
                    owner: "near".to_string(),
                    repo: "ironclaw".to_string(),
                    number: 42,
                    node_id: None,
                    url: "https://github.com/near/ironclaw/issues/42".to_string(),
                    default_branch: "main".to_string(),
                },
            })
            .await
            .expect("project access allowed");

        let captured = project_service.captured_get_projects();
        assert_eq!(captured.len(), 2);
        for (caller, request) in captured {
            assert_eq!(caller.tenant_id.as_str(), "workflow-tenant");
            assert_eq!(caller.user_id.as_str(), "workflow-owner");
            assert_eq!(request.project_id, "workflow-project");
        }
    }

    #[tokio::test]
    async fn assert_workflow_config_access_rejects_disallowed_repo_and_account() {
        let access = ProjectServiceWorkflowProjectAccess {
            project_service: Arc::new(FakeProjectService::new(json!({
                "github_issue_workflow": {
                    "enabled": true,
                    "repositories": [
                        { "owner": "nearai", "repo": "ironclaw" }
                    ]
                }
            }))),
            configured_provider_account_ref: GithubProviderAccountRef {
                provider: "github".to_string(),
                account_id: "runtime-github-account".to_string(),
            },
        };
        let request = |repositories, provider_account_ref| WorkflowConfigAccessRequest {
            tenant_id: TenantId::new("workflow-tenant").expect("tenant"),
            creator_user_id: UserId::new("workflow-owner").expect("user"),
            project_id: ProjectId::new("workflow-project").expect("project"),
            repositories,
            provider_account_ref,
        };
        let valid_account = || GithubProviderAccountRef {
            provider: "github".to_string(),
            account_id: "runtime-github-account".to_string(),
        };
        let valid_repo = || {
            ironclaw_github_issue_workflow::GithubRepositorySelector::new("nearai", "ironclaw")
                .expect("selector")
        };

        // (a) no repositories -> fail closed.
        let empty = access
            .assert_workflow_config_access(request(Vec::new(), valid_account()))
            .await
            .expect_err("empty repositories must be denied");
        assert!(
            matches!(empty, GithubIssueWorkflowError::PolicyDenied { .. }),
            "expected PolicyDenied, got {empty:?}"
        );

        // (b) malformed repository selector -> fail closed (bypass ::new validation).
        let bad_selector = access
            .assert_workflow_config_access(request(
                vec![ironclaw_github_issue_workflow::GithubRepositorySelector {
                    owner: String::new(),
                    repo: "ironclaw".to_string(),
                }],
                valid_account(),
            ))
            .await
            .expect_err("invalid repository selector must be denied");
        assert!(
            matches!(bad_selector, GithubIssueWorkflowError::PolicyDenied { .. }),
            "expected PolicyDenied, got {bad_selector:?}"
        );

        // (c) malformed provider account ref -> fail closed.
        let bad_account = access
            .assert_workflow_config_access(request(
                vec![valid_repo()],
                GithubProviderAccountRef {
                    provider: String::new(),
                    account_id: "x".to_string(),
                },
            ))
            .await
            .expect_err("invalid provider account must be denied");
        assert!(
            matches!(bad_account, GithubIssueWorkflowError::PolicyDenied { .. }),
            "expected PolicyDenied, got {bad_account:?}"
        );

        // Happy path: valid repositories + account is allowed.
        access
            .assert_workflow_config_access(request(vec![valid_repo()], valid_account()))
            .await
            .expect("valid config access is allowed");
    }

    #[tokio::test]
    async fn poller_uses_project_service_access_before_github_reads() {
        let project_service = Arc::new(FakeProjectService::deny_after_get_projects(
            json!({
                "github_issue_workflow": {
                    "enabled": true,
                    "repositories": [
                        { "owner": "near", "repo": "ironclaw" }
                    ]
                }
            }),
            1,
        ));
        let source = Arc::new(source_with_service(project_service.clone()));
        let access = Arc::new(ProjectServiceWorkflowProjectAccess {
            project_service: project_service.clone(),
            configured_provider_account_ref: GithubProviderAccountRef {
                provider: "github".to_string(),
                account_id: "runtime-github-account".to_string(),
            },
        });
        let github = Arc::new(CountingGithubPort::default());
        let poller = GithubIssueWorkflowPoller::new(
            IronClawGithubIssueWorkflowPollerPorts {
                clock: Arc::new(TestWorkflowClock),
                config_source: source,
                github_port: github.clone(),
                project_access: access,
                repository: Arc::new(InMemoryGithubIssueWorkflowRepository::default()),
                stage_turn_submitter: Arc::new(NoopStageTurnSubmitter),
                workspace_manager: test_only_unconfigured_workspace_manager(),
                worker_id: WorkflowWorkerId::new(),
            },
            GithubIssueWorkflowPollerConfig {
                enabled: true,
                poll_interval: Duration::from_secs(60),
                max_repos_per_tick: 10,
                max_issues_per_repo_per_tick: 10,
                max_runnable_runs_per_tick: 10,
                lease_duration: Duration::from_secs(300),
                stage_stale_after: Duration::from_secs(1800),
            },
            "project-access-caller-level-test",
        );

        let outcome = poller.tick_once().await.expect("poller tick");

        assert_eq!(outcome.configs_loaded, 1);
        assert_eq!(outcome.repositories_scanned, 0);
        assert_eq!(outcome.blocked_configs.len(), 1);
        assert_eq!(
            outcome.blocked_configs[0].kind,
            GithubIssueWorkflowPollerBlockKind::ProjectAccessDenied
        );
        assert_eq!(
            github.search_calls(),
            0,
            "GitHub search must not happen before project access succeeds"
        );
        let captured = project_service.captured_get_projects();
        assert_eq!(
            captured.len(),
            2,
            "metadata load and access gate should both use ProjectService"
        );
    }

    #[tokio::test]
    async fn project_service_project_access_denies_missing_project_scope() {
        let access = ProjectServiceWorkflowProjectAccess {
            project_service: Arc::new(FakeProjectService::new(json!({}))),
            configured_provider_account_ref: GithubProviderAccountRef {
                provider: "github".to_string(),
                account_id: "runtime-github-account".to_string(),
            },
        };

        let error = access
            .assert_workflow_project_access(WorkflowProjectAccessRequest {
                tenant_id: TenantId::new("workflow-tenant").expect("tenant"),
                creator_user_id: UserId::new("workflow-owner").expect("user"),
                agent_id: Some(AgentId::new("workflow-agent").expect("agent")),
                project_id: None,
                workflow_run_id:
                    ironclaw_github_issue_workflow::GithubIssueWorkflowRunId::from_trusted(
                        "workflow-run-no-project".to_string(),
                    )
                    .expect("workflow run"),
                issue: GithubIssueRef {
                    owner: "near".to_string(),
                    repo: "ironclaw".to_string(),
                    number: 42,
                    node_id: None,
                    url: "https://github.com/near/ironclaw/issues/42".to_string(),
                    default_branch: "main".to_string(),
                },
            })
            .await
            .expect_err("missing project scope denied");

        assert!(matches!(
            error,
            GithubIssueWorkflowError::PolicyDenied { .. }
        ));
    }

    #[tokio::test]
    async fn runtime_workspace_manager_rejects_unsafe_repository_components_before_git() {
        let root = tempfile::tempdir().expect("tempdir");
        let manager = RuntimeWorkflowWorkspaceManager {
            local_dev_storage_root: root.path().to_path_buf(),
            git_remote: WorkflowGitRemoteConfig::local_dev_default(),
        };
        let error = manager
            .prepare_workspace(PrepareWorkflowWorkspaceRequest {
                tenant_id: TenantId::new("workflow-tenant").expect("tenant"),
                creator_user_id: UserId::new("workflow-owner").expect("user"),
                agent_id: Some(AgentId::new("workflow-agent").expect("agent")),
                project_id: Some(ProjectId::new("workflow-project").expect("project")),
                workflow_run_id:
                    ironclaw_github_issue_workflow::GithubIssueWorkflowRunId::from_trusted(
                        "workflow-run-1234567890".to_string(),
                    )
                    .expect("workflow run"),
                issue: GithubIssueRef {
                    owner: "near/evil".to_string(),
                    repo: "ironclaw".to_string(),
                    number: 42,
                    node_id: None,
                    url: "https://github.com/near/ironclaw/issues/42".to_string(),
                    default_branch: "main".to_string(),
                },
                base_branch: "main".to_string(),
                requested_at: Utc::now(),
            })
            .await
            .expect_err("unsafe repository owner should fail before git clone");

        assert!(matches!(
            error,
            GithubIssueWorkflowError::InvalidConfig { ref reason }
                if reason.contains("repository owner")
        ));
        assert!(!root.path().join("github-issue-workspaces").exists());
    }

    #[tokio::test]
    async fn runtime_workspace_manager_prepare_then_publish_with_empty_base_branch_resolves_default_and_pushes()
     {
        // Regression for the live-E2E bugs the hermetic provider fixture hid by
        // hardcoding default_branch:"main". Live GitHub payloads omit
        // default_branch, so base_branch arrives EMPTY: prepare must resolve the
        // remote's real default (here "trunk", not "" and not "main"), and
        // publish must detect the new commit and actually push (not silently skip).
        fn git(args: &[&str], dir: &std::path::Path) {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .status()
                .expect("run git");
            assert!(status.success(), "git {args:?} failed");
        }

        let tmp = tempfile::tempdir().expect("tempdir");
        let owner = "ironclaw-e2e";
        let repo = "fixture";
        // Bare remote at <tmp>/<owner>/<repo>.git so the clone URL
        // file://<tmp>/<owner>/<repo>.git resolves; default branch = "trunk".
        let bare = tmp.path().join(owner).join(format!("{repo}.git"));
        std::fs::create_dir_all(&bare).expect("bare parent");
        git(&["init", "--bare", bare.to_str().unwrap()], tmp.path());
        let seed = tmp.path().join("seed");
        git(
            &["clone", bare.to_str().unwrap(), seed.to_str().unwrap()],
            tmp.path(),
        );
        git(&["checkout", "-b", "trunk"], &seed);
        std::fs::write(seed.join("README.md"), "fixture\n").expect("seed file");
        git(&["add", "."], &seed);
        git(&["commit", "-m", "seed"], &seed);
        git(&["push", "-u", "origin", "trunk"], &seed);
        // Make "trunk" the remote default so a clone-without-branch and
        // origin/HEAD resolve to it (do not rely on init.defaultBranch).
        git(&["symbolic-ref", "HEAD", "refs/heads/trunk"], &bare);

        let storage = tmp.path().join("storage");
        std::fs::create_dir_all(&storage).expect("storage");
        let manager = RuntimeWorkflowWorkspaceManager {
            local_dev_storage_root: storage.clone(),
            git_remote: WorkflowGitRemoteConfig {
                config_args: Vec::new(),
                clone_base_url: format!("file://{}", tmp.path().display()),
                committer_name: "IronClaw Bot".to_string(),
                committer_email: "bot@ironclaw.test".to_string(),
            },
        };
        let issue = GithubIssueRef {
            owner: owner.to_string(),
            repo: repo.to_string(),
            number: 7,
            node_id: None,
            url: format!("https://example.invalid/{owner}/{repo}/issues/7"),
            // The empty default_branch the live GitHub payload yields.
            default_branch: String::new(),
        };
        let run_id = ironclaw_github_issue_workflow::GithubIssueWorkflowRunId::from_trusted(
            "workflow-run-empty-branch".to_string(),
        )
        .expect("workflow run");

        let prepared = manager
            .prepare_workspace(PrepareWorkflowWorkspaceRequest {
                tenant_id: TenantId::new("t").expect("tenant"),
                creator_user_id: UserId::new("u").expect("user"),
                agent_id: Some(AgentId::new("a").expect("agent")),
                project_id: Some(ProjectId::new("p").expect("project")),
                workflow_run_id: run_id.clone(),
                issue: issue.clone(),
                base_branch: String::new(),
                requested_at: Utc::now(),
            })
            .await
            .expect("prepare workspace");
        assert_eq!(
            prepared.session.base_branch, "trunk",
            "empty base must resolve to the remote default branch, not \"\" or \"main\""
        );

        let session_id = prepared.session.workspace_session_id.clone();
        let checkout = workflow_workspace_host_path(&storage, &session_id);
        std::fs::write(checkout.join("fix.txt"), "the fix\n").expect("write change");

        let published = manager
            .publish_workspace(PublishWorkflowWorkspaceRequest {
                tenant_id: TenantId::new("t").expect("tenant"),
                creator_user_id: UserId::new("u").expect("user"),
                agent_id: Some(AgentId::new("a").expect("agent")),
                project_id: Some(ProjectId::new("p").expect("project")),
                workflow_run_id: run_id,
                issue,
                workspace_session_id: session_id,
                base_branch: String::new(),
                commit_message: "ironclaw: apply fix".to_string(),
                requested_at: Utc::now(),
            })
            .await
            .expect("publish workspace");

        assert!(
            published.has_changes,
            "publish must detect the new commit and push, not silently skip"
        );
        assert_eq!(published.base_branch, "trunk");

        let remote_refs = std::process::Command::new("git")
            .args([
                "--git-dir",
                bare.to_str().unwrap(),
                "for-each-ref",
                "--format=%(refname)",
                "refs/heads/",
            ])
            .output()
            .expect("list remote refs");
        let remote_refs = String::from_utf8_lossy(&remote_refs.stdout);
        assert!(
            remote_refs.contains(&published.working_branch),
            "pushed working branch {} not found in remote refs:\n{remote_refs}",
            published.working_branch
        );
    }

    #[tokio::test]
    async fn runtime_workspace_manager_verify_reports_exit_and_publish_excludes_bytecode() {
        // Real host-process verification: an explicit command's exit code is
        // reported faithfully; no configured/detected runner skips the gate; and
        // build caches (__pycache__) seeded into the checkout are kept out of the
        // published commit.
        fn git(args: &[&str], dir: &std::path::Path) {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .status()
                .expect("run git");
            assert!(status.success(), "git {args:?} failed");
        }

        let tmp = tempfile::tempdir().expect("tempdir");
        let owner = "ironclaw-e2e";
        let repo = "verify-fixture";
        let bare = tmp.path().join(owner).join(format!("{repo}.git"));
        std::fs::create_dir_all(&bare).expect("bare parent");
        git(&["init", "--bare", bare.to_str().unwrap()], tmp.path());
        let seed = tmp.path().join("seed");
        git(
            &["clone", bare.to_str().unwrap(), seed.to_str().unwrap()],
            tmp.path(),
        );
        git(&["checkout", "-b", "trunk"], &seed);
        std::fs::write(seed.join("README.md"), "fixture\n").expect("seed file");
        git(&["add", "."], &seed);
        git(&["commit", "-m", "seed"], &seed);
        git(&["push", "-u", "origin", "trunk"], &seed);
        git(&["symbolic-ref", "HEAD", "refs/heads/trunk"], &bare);

        let storage = tmp.path().join("storage");
        std::fs::create_dir_all(&storage).expect("storage");
        let manager = RuntimeWorkflowWorkspaceManager {
            local_dev_storage_root: storage.clone(),
            git_remote: WorkflowGitRemoteConfig {
                config_args: Vec::new(),
                clone_base_url: format!("file://{}", tmp.path().display()),
                committer_name: "IronClaw Bot".to_string(),
                committer_email: "bot@ironclaw.test".to_string(),
            },
        };
        let issue = GithubIssueRef {
            owner: owner.to_string(),
            repo: repo.to_string(),
            number: 9,
            node_id: None,
            url: format!("https://example.invalid/{owner}/{repo}/issues/9"),
            default_branch: String::new(),
        };
        let run_id = ironclaw_github_issue_workflow::GithubIssueWorkflowRunId::from_trusted(
            "workflow-run-verify".to_string(),
        )
        .expect("workflow run");

        let prepared = manager
            .prepare_workspace(PrepareWorkflowWorkspaceRequest {
                tenant_id: TenantId::new("t").expect("tenant"),
                creator_user_id: UserId::new("u").expect("user"),
                agent_id: Some(AgentId::new("a").expect("agent")),
                project_id: Some(ProjectId::new("p").expect("project")),
                workflow_run_id: run_id.clone(),
                issue: issue.clone(),
                base_branch: String::new(),
                requested_at: Utc::now(),
            })
            .await
            .expect("prepare workspace");
        let session_id = prepared.session.workspace_session_id.clone();

        let verify_request =
            |command: Option<WorkflowVerificationCommand>| VerifyWorkflowWorkspaceRequest {
                tenant_id: TenantId::new("t").expect("tenant"),
                creator_user_id: UserId::new("u").expect("user"),
                agent_id: Some(AgentId::new("a").expect("agent")),
                project_id: Some(ProjectId::new("p").expect("project")),
                workflow_run_id: run_id.clone(),
                issue: issue.clone(),
                workspace_session_id: session_id.clone(),
                command,
                requested_at: Utc::now(),
            };

        // Explicit command exit 0 -> ran + passed.
        let passing = manager
            .verify_workspace(verify_request(Some(WorkflowVerificationCommand {
                program: "true".to_string(),
                args: Vec::new(),
                timeout_secs: 30,
            })))
            .await
            .expect("verify true");
        assert!(passing.ran && passing.passed, "`true` must report passed");

        // Explicit command exit non-zero -> ran + NOT passed (a policy decision,
        // not an Err).
        let failing = manager
            .verify_workspace(verify_request(Some(WorkflowVerificationCommand {
                program: "false".to_string(),
                args: Vec::new(),
                timeout_secs: 30,
            })))
            .await
            .expect("verify false is Ok, not Err");
        assert!(failing.ran && !failing.passed, "`false` must report failed");

        // No command + no detectable runner (README-only repo) -> skipped.
        let skipped = manager
            .verify_workspace(verify_request(None))
            .await
            .expect("verify auto-detect");
        assert!(
            !skipped.ran && skipped.passed,
            "no detected runner must skip the gate (ran: false)"
        );

        // Bytecode exclusion: a __pycache__ artifact in the checkout must NOT be
        // committed/pushed by publish_workspace.
        let checkout = workflow_workspace_host_path(&storage, &session_id);
        std::fs::write(checkout.join("fix.txt"), "the fix\n").expect("write change");
        std::fs::create_dir_all(checkout.join("__pycache__")).expect("pycache dir");
        std::fs::write(checkout.join("__pycache__").join("x.pyc"), b"\x00bytecode")
            .expect("write pyc");

        manager
            .publish_workspace(PublishWorkflowWorkspaceRequest {
                tenant_id: TenantId::new("t").expect("tenant"),
                creator_user_id: UserId::new("u").expect("user"),
                agent_id: Some(AgentId::new("a").expect("agent")),
                project_id: Some(ProjectId::new("p").expect("project")),
                workflow_run_id: run_id,
                issue,
                workspace_session_id: session_id,
                base_branch: String::new(),
                commit_message: "ironclaw: apply fix".to_string(),
                requested_at: Utc::now(),
            })
            .await
            .expect("publish workspace");

        let committed = std::process::Command::new("git")
            .args([
                "-C",
                checkout.to_str().unwrap(),
                "ls-tree",
                "-r",
                "--name-only",
                "HEAD",
            ])
            .output()
            .expect("ls-tree");
        let committed = String::from_utf8_lossy(&committed.stdout);
        assert!(
            committed.contains("fix.txt"),
            "the fix must be committed:\n{committed}"
        );
        assert!(
            !committed.contains("__pycache__") && !committed.contains(".pyc"),
            "build artifacts must be excluded from the commit:\n{committed}"
        );
    }

    #[test]
    fn workflow_stage_workspace_metadata_builds_session_mount_view() {
        let metadata = json!({
            "kind": "github_issue_workflow_stage",
            "workspace_mount_ref": {
                "mount_id": "11111111-1111-4111-8111-111111111111",
                "alias": "/workspace"
            }
        })
        .to_string();

        let mounts = workflow_stage_workspace_mount_view_from_thread_metadata(&metadata)
            .expect("metadata parses")
            .expect("workflow mount view");

        assert_eq!(mounts.mounts.len(), 1);
        assert_eq!(mounts.mounts[0].alias.as_str(), "/workspace");
        assert_eq!(
            mounts.mounts[0].target.as_str(),
            "/projects/github-issue-workspaces/11111111-1111-4111-8111-111111111111"
        );
        assert!(mounts.mounts[0].permissions.write);
    }

    #[test]
    fn git_branch_component_replaces_unsafe_characters() {
        assert_eq!(git_branch_component("near/iron claw"), "near-iron-claw");
    }

    fn source_with_service(
        project_service: Arc<FakeProjectService>,
    ) -> ProjectMetadataGithubIssueWorkflowConfigSource {
        ProjectMetadataGithubIssueWorkflowConfigSource {
            project_service,
            tenant_id: TenantId::new("workflow-tenant").expect("tenant"),
            owner_user_id: UserId::new("workflow-owner").expect("user"),
            project_id: ProjectId::new("workflow-project").expect("project"),
            configured_provider_account_ref: GithubProviderAccountRef {
                provider: "github".to_string(),
                account_id: "runtime-github-account".to_string(),
            },
        }
    }

    struct FakeProjectService {
        metadata: JsonValue,
        successful_get_projects_before_denial: Option<usize>,
        captured_get_projects: Mutex<Vec<(ProjectCaller, RebornGetProjectRequest)>>,
    }

    impl FakeProjectService {
        fn new(metadata: JsonValue) -> Self {
            Self {
                metadata,
                successful_get_projects_before_denial: None,
                captured_get_projects: Mutex::new(Vec::new()),
            }
        }

        fn deny_after_get_projects(
            metadata: JsonValue,
            successful_get_projects_before_denial: usize,
        ) -> Self {
            Self {
                metadata,
                successful_get_projects_before_denial: Some(successful_get_projects_before_denial),
                captured_get_projects: Mutex::new(Vec::new()),
            }
        }

        fn captured_get_project(&self) -> (ProjectCaller, RebornGetProjectRequest) {
            self.captured_get_projects()
                .pop()
                .expect("get_project captured")
        }

        fn captured_get_projects(&self) -> Vec<(ProjectCaller, RebornGetProjectRequest)> {
            self.captured_get_projects.lock().expect("lock").clone()
        }
    }

    #[async_trait]
    impl ProjectService for FakeProjectService {
        async fn list_projects(
            &self,
            _caller: ProjectCaller,
            _request: RebornListProjectsRequest,
        ) -> Result<RebornListProjectsResponse, ProjectServiceError> {
            panic!("list_projects is not used by these tests")
        }

        async fn create_project(
            &self,
            _caller: ProjectCaller,
            _request: RebornCreateProjectRequest,
        ) -> Result<RebornProjectResponse, ProjectServiceError> {
            panic!("create_project is not used by these tests")
        }

        async fn get_project(
            &self,
            caller: ProjectCaller,
            request: RebornGetProjectRequest,
        ) -> Result<RebornProjectResponse, ProjectServiceError> {
            let call_count = {
                let mut captured = self.captured_get_projects.lock().expect("lock");
                captured.push((caller, request.clone()));
                captured.len()
            };
            if self
                .successful_get_projects_before_denial
                .is_some_and(|allowed| call_count > allowed)
            {
                return Err(ProjectServiceError::Denied);
            }
            Ok(RebornProjectResponse {
                project: RebornProjectInfo {
                    project_id: request.project_id,
                    name: "Workflow project".to_string(),
                    description: String::new(),
                    icon: None,
                    color: None,
                    metadata: self.metadata.clone(),
                    state: RebornProjectState::Active,
                    role: RebornProjectRole::Owner,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
            })
        }

        async fn update_project(
            &self,
            _caller: ProjectCaller,
            _request: RebornUpdateProjectRequest,
        ) -> Result<RebornProjectResponse, ProjectServiceError> {
            panic!("update_project is not used by these tests")
        }

        async fn delete_project(
            &self,
            _caller: ProjectCaller,
            _request: RebornDeleteProjectRequest,
        ) -> Result<(), ProjectServiceError> {
            panic!("delete_project is not used by these tests")
        }

        async fn list_members(
            &self,
            _caller: ProjectCaller,
            _request: RebornListMembersRequest,
        ) -> Result<RebornListMembersResponse, ProjectServiceError> {
            panic!("list_members is not used by these tests")
        }

        async fn add_member(
            &self,
            _caller: ProjectCaller,
            _request: RebornAddMemberRequest,
        ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
            panic!("add_member is not used by these tests")
        }

        async fn update_member_role(
            &self,
            _caller: ProjectCaller,
            _request: RebornUpdateMemberRoleRequest,
        ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
            panic!("update_member_role is not used by these tests")
        }

        async fn remove_member(
            &self,
            _caller: ProjectCaller,
            _request: RebornRemoveMemberRequest,
        ) -> Result<(), ProjectServiceError> {
            panic!("remove_member is not used by these tests")
        }
    }

    #[derive(Default)]
    struct CountingGithubPort {
        search_calls: Mutex<usize>,
    }

    impl CountingGithubPort {
        fn search_calls(&self) -> usize {
            *self.search_calls.lock().expect("lock")
        }
    }

    #[async_trait]
    impl GithubIssueWorkflowPort for CountingGithubPort {
        async fn search_open_bug_issues(
            &self,
            _input: SearchGithubIssuesInput,
        ) -> Result<Vec<GithubIssueSearchHit>, GithubIssueWorkflowError> {
            *self.search_calls.lock().expect("lock") += 1;
            Ok(Vec::new())
        }

        async fn get_authenticated_workflow_actor(
            &self,
            _input: GetAuthenticatedWorkflowActorInput,
        ) -> Result<GithubActorSnapshot, GithubIssueWorkflowError> {
            panic!("get_authenticated_workflow_actor is not used by this test")
        }

        async fn list_issue_comments(
            &self,
            _input: ListIssueCommentsInput,
        ) -> Result<Vec<GithubIssueCommentSnapshot>, GithubIssueWorkflowError> {
            panic!("list_issue_comments is not used by this test")
        }

        async fn create_issue_comment(
            &self,
            _input: CreateIssueCommentInput,
        ) -> Result<GithubCommentRef, GithubIssueWorkflowError> {
            panic!("create_issue_comment is not used by this test")
        }
    }

    struct NoopStageTurnSubmitter;

    #[async_trait]
    impl StageTurnSubmitter for NoopStageTurnSubmitter {
        async fn submit_stage_turn(
            &self,
            _request: SubmitStageTurnRequest,
        ) -> Result<SubmitStageTurnOutcome, GithubIssueWorkflowError> {
            panic!("submit_stage_turn is not used by this test")
        }
    }

    struct TestWorkflowClock;

    #[async_trait]
    impl WorkflowClock for TestWorkflowClock {
        fn now(&self) -> chrono::DateTime<Utc> {
            Utc::now()
        }
    }
}

pub(crate) struct IronClawStageTurnSubmitter {
    thread_service: Arc<dyn SessionThreadService>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    actor_user_id: UserId,
    default_agent_id: AgentId,
}

impl IronClawStageTurnSubmitter {
    pub(crate) fn new(
        thread_service: Arc<dyn SessionThreadService>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
        actor_user_id: UserId,
        default_agent_id: AgentId,
    ) -> Self {
        Self {
            thread_service,
            turn_coordinator,
            actor_user_id,
            default_agent_id,
        }
    }

    fn thread_scope(&self, scope: &WorkflowActorScope) -> ThreadScope {
        ThreadScope {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope
                .agent_id
                .clone()
                .unwrap_or_else(|| self.default_agent_id.clone()),
            project_id: scope.project_id.clone(),
            owner_user_id: Some(scope.creator_user_id.clone()),
            mission_id: None,
        }
    }

    fn actor_id(&self) -> String {
        self.actor_user_id.as_str().to_string()
    }
}

#[async_trait]
impl StageTurnSubmitter for IronClawStageTurnSubmitter {
    async fn submit_stage_turn(
        &self,
        request: SubmitStageTurnRequest,
    ) -> Result<SubmitStageTurnOutcome, GithubIssueWorkflowError> {
        let thread_scope = self.thread_scope(&request.scope);
        let thread_id = deterministic_stage_thread_id(&request)?;
        let source_binding_id = request.stage_turn_identity.source_binding_ref();
        let external_event_id = request.idempotency_key.as_str().to_string();

        let thread = self
            .thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id),
                created_by_actor_id: self.actor_id(),
                title: Some(stage_thread_title(&request.stage_turn_identity.stage).to_string()),
                metadata_json: Some(stage_thread_metadata(&request)?),
            })
            .await
            .map_err(map_thread_error)?;

        if let Some(replay) = self
            .thread_service
            .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
                scope: thread_scope.clone(),
                actor_id: self.actor_id(),
                source_binding_id: source_binding_id.clone(),
                external_event_id: external_event_id.clone(),
            })
            .await
            .map_err(map_thread_error)?
        {
            match replay.status {
                MessageStatus::Submitted => {
                    if let Some(turn_run_id) = replay.turn_run_id.as_deref() {
                        return Ok(SubmitStageTurnOutcome::Replayed {
                            thread_id: replay.thread_id,
                            turn_run_id: parse_turn_run_id(turn_run_id)?,
                        });
                    }
                    return Err(GithubIssueWorkflowError::Policy {
                        reason: "submitted stage turn message is missing turn_run_id".to_string(),
                    });
                }
                MessageStatus::RejectedBusy => {
                    return Ok(SubmitStageTurnOutcome::Busy {
                        reason:
                            "stage turn message was already rejected because the thread was busy"
                                .to_string(),
                    });
                }
                MessageStatus::Accepted => {
                    return self
                        .submit_accepted_message(
                            request,
                            thread_scope,
                            replay.thread_id,
                            replay.message_id,
                        )
                        .await;
                }
                _ => {}
            }
        }

        let accepted = self
            .thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread.thread_id,
                actor_id: self.actor_id(),
                source_binding_id: Some(source_binding_id),
                reply_target_binding_id: Some(
                    request.stage_turn_identity.reply_target_binding_ref(),
                ),
                external_event_id: Some(external_event_id),
                content: MessageContent::text(request.prompt.content.clone()),
            })
            .await
            .map_err(map_thread_error)?;

        self.submit_accepted_message(
            request,
            thread_scope,
            accepted.thread_id,
            accepted.message_id,
        )
        .await
    }
}

impl IronClawStageTurnSubmitter {
    async fn submit_accepted_message(
        &self,
        request: SubmitStageTurnRequest,
        thread_scope: ThreadScope,
        thread_id: ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<SubmitStageTurnOutcome, GithubIssueWorkflowError> {
        let turn_scope = TurnScope::new_with_owner(
            thread_scope.tenant_id.clone(),
            Some(thread_scope.agent_id.clone()),
            thread_scope.project_id.clone(),
            thread_id.clone(),
            thread_scope.owner_user_id.clone(),
        );
        let actor = TurnActor::new(self.actor_user_id.clone());
        let accepted_message_ref = accepted_message_ref(message_id)?;
        let source_binding_ref =
            SourceBindingRef::new(request.stage_turn_identity.source_binding_ref())
                .map_err(invalid_ref)?;
        let reply_target_binding_ref =
            ReplyTargetBindingRef::new(request.stage_turn_identity.reply_target_binding_ref())
                .map_err(invalid_ref)?;
        let requested_run_profile = RunProfileRequest::new(stage_capability_profile_id(
            &request.stage_turn_identity.stage,
        ))
        .map_err(invalid_ref)?;
        let idempotency_key = IdempotencyKey::new(request.idempotency_key.as_str().to_string())
            .map_err(invalid_ref)?;
        let product_context = workflow_product_context(&turn_scope, &actor)?;

        let submit_result = self
            .turn_coordinator
            .submit_turn(SubmitTurnRequest {
                scope: turn_scope,
                actor,
                accepted_message_ref,
                source_binding_ref,
                reply_target_binding_ref,
                requested_run_profile: Some(requested_run_profile),
                idempotency_key,
                received_at: Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: Some(product_context),
            })
            .await;

        match submit_result {
            Ok(SubmitTurnResponse::Accepted {
                turn_id, run_id, ..
            }) => {
                self.thread_service
                    .mark_message_submitted(
                        &thread_scope,
                        &thread_id,
                        message_id,
                        turn_id.to_string(),
                        run_id.to_string(),
                    )
                    .await
                    .map_err(map_thread_error)?;
                Ok(SubmitStageTurnOutcome::Submitted {
                    thread_id,
                    turn_run_id: run_id,
                })
            }
            Err(TurnError::ThreadBusy(busy)) => {
                self.thread_service
                    .mark_message_rejected_busy(&thread_scope, &thread_id, message_id)
                    .await
                    .map_err(map_thread_error)?;
                Ok(SubmitStageTurnOutcome::Busy {
                    reason: format!(
                        "thread already has active run {} with status {:?}",
                        busy.active_run_id, busy.status
                    ),
                })
            }
            Err(error) => Err(map_turn_error(error)),
        }
    }
}

fn deterministic_stage_thread_id(
    request: &SubmitStageTurnRequest,
) -> Result<ThreadId, GithubIssueWorkflowError> {
    ThreadId::new(request.stage_turn_identity.thread_id_seed()).map_err(|error| {
        GithubIssueWorkflowError::Policy {
            reason: format!("invalid deterministic stage thread id: {error}"),
        }
    })
}

fn accepted_message_ref(
    message_id: ThreadMessageId,
) -> Result<AcceptedMessageRef, GithubIssueWorkflowError> {
    AcceptedMessageRef::new(message_id.to_string()).map_err(invalid_ref)
}

fn parse_turn_run_id(value: &str) -> Result<TurnRunId, GithubIssueWorkflowError> {
    TurnRunId::parse(value).map_err(|error| GithubIssueWorkflowError::Policy {
        reason: format!("invalid replayed turn run id: {error}"),
    })
}

fn workflow_product_context(
    turn_scope: &TurnScope,
    actor: &TurnActor,
) -> Result<ProductTurnContext, GithubIssueWorkflowError> {
    let adapter = RunOriginAdapter::new(WORKFLOW_ADAPTER_ID).map_err(map_turn_error)?;
    Ok(ironclaw_product_context::resolve_inbound(
        InboundClassification::TrustedOther,
        adapter,
        Some(TurnSurfaceType::Direct),
        turn_scope.product_owner(actor),
    ))
}

fn stage_thread_title(stage: &GithubIssueStage) -> &'static str {
    match stage {
        GithubIssueStage::Triage => "GitHub issue workflow: triage",
        GithubIssueStage::Planning => "GitHub issue workflow: planning",
        GithubIssueStage::Implementation => "GitHub issue workflow: implementation",
        GithubIssueStage::PrSynthesis => "GitHub issue workflow: PR synthesis",
        GithubIssueStage::CiRepair => "GitHub issue workflow: CI repair",
        GithubIssueStage::ReviewResponse => "GitHub issue workflow: review response",
    }
}

fn stage_label(stage: &GithubIssueStage) -> &'static str {
    match stage {
        GithubIssueStage::Triage => "triage",
        GithubIssueStage::Planning => "planning",
        GithubIssueStage::Implementation => "implementation",
        GithubIssueStage::PrSynthesis => "pr_synthesis",
        GithubIssueStage::CiRepair => "ci_repair",
        GithubIssueStage::ReviewResponse => "review_response",
    }
}

fn stage_thread_metadata(
    request: &SubmitStageTurnRequest,
) -> Result<String, GithubIssueWorkflowError> {
    serde_json::to_string(&json!({
        "kind": GITHUB_ISSUE_WORKFLOW_STAGE_THREAD_KIND,
        "workflow_run_id": request.stage_turn_identity.workflow_run_id.as_str(),
        "stage_run_id": request.stage_turn_identity.stage_run_id.as_str(),
        "stage": stage_label(&request.stage_turn_identity.stage),
        "attempt": request.stage_turn_identity.attempt,
        "workflow_policy_version": request.stage_turn_identity.workflow_policy_version.as_str(),
        "prompt_ref": request.prompt.content_ref.prompt_ref.as_str(),
        "prompt_version": request.prompt.content_ref.prompt_version.as_str(),
        "input_snapshot_hash": request.prompt.content_ref.input_snapshot_hash.as_str(),
        "prompt_content_hash": request.prompt.content_hash.as_str(),
        "workspace_mount_ref": request.workspace_mount_ref.as_ref().map(|mount| json!({
            "mount_id": mount.mount_id.as_str(),
            "alias": mount.alias.as_str(),
        })),
    }))
    .map_err(|error| GithubIssueWorkflowError::Policy {
        reason: format!("failed to serialize stage thread metadata: {error}"),
    })
}

pub(crate) fn workflow_stage_workspace_mount_view_from_thread_metadata(
    metadata_json: &str,
) -> Result<Option<MountView>, GithubIssueWorkflowError> {
    let metadata: JsonValue =
        serde_json::from_str(metadata_json).map_err(|error| GithubIssueWorkflowError::Policy {
            reason: format!("failed to parse GitHub issue workflow stage metadata: {error}"),
        })?;
    if metadata.get("kind").and_then(JsonValue::as_str)
        != Some(GITHUB_ISSUE_WORKFLOW_STAGE_THREAD_KIND)
    {
        return Ok(None);
    }
    let Some(mount_ref) = metadata.get("workspace_mount_ref") else {
        return Ok(None);
    };
    if mount_ref.is_null() {
        return Ok(None);
    }
    let mount_id = mount_ref
        .get("mount_id")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| GithubIssueWorkflowError::Policy {
            reason: "GitHub issue workflow stage metadata is missing workspace mount_id"
                .to_string(),
        })?;
    let alias = mount_ref
        .get("alias")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| GithubIssueWorkflowError::Policy {
            reason: "GitHub issue workflow stage metadata is missing workspace mount alias"
                .to_string(),
        })?;
    if alias != crate::local_dev_mounts::WORKSPACE_ALIAS {
        return Err(GithubIssueWorkflowError::Policy {
            reason: "GitHub issue workflow stage workspace mount alias is not /workspace"
                .to_string(),
        });
    }
    Uuid::parse_str(mount_id).map_err(|error| GithubIssueWorkflowError::Policy {
        reason: format!("GitHub issue workflow workspace mount id is not a UUID: {error}"),
    })?;
    let workspace_session_id = GithubIssueWorkspaceSessionId::from_trusted(mount_id.to_string())?;
    workflow_workspace_mount_view(&workspace_session_id, alias).map(Some)
}

fn map_thread_error(error: SessionThreadError) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::Policy {
        reason: format!("stage turn thread operation failed: {error}"),
    }
}

fn map_turn_error(error: TurnError) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::Policy {
        reason: format!("stage turn submit failed: {error}"),
    }
}

fn invalid_ref(error: impl std::fmt::Display) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::Policy {
        reason: format!("invalid stage turn request reference: {error}"),
    }
}

#[cfg(test)]
mod github_issue_workflow_provider_runtime_contract_tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_github_issue_workflow::{
        GithubIssueCandidateSelector, GithubIssueRef, GithubIssueWorkflowConfig,
        GithubIssueWorkflowConfigSource, GithubIssueWorkflowError, GithubIssueWorkflowPort,
        GithubIssueWorkflowRunId, GithubProviderAccountRef, GithubRepositorySelector,
        PrepareWorkflowWorkspaceRequest, SearchGithubIssuesInput, WorkflowConfigAccessRequest,
        WorkflowProjectAccess, WorkflowProjectAccessRequest, WorkflowWorkspaceManager,
    };
    use ironclaw_host_api::{
        AgentId, CapabilitySet, EffectKind, ExecutionContext, ExtensionId, MountView,
        NetworkScheme, NetworkTargetPattern, ProjectId, ResourceUsage, RuntimeKind, TenantId,
        TrustClass, UserId,
    };
    use ironclaw_host_runtime::{
        CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, HostRuntime, HostRuntimeError,
        HostRuntimeHealth, HostRuntimeStatus, RuntimeCapabilityAuthResumeRequest,
        RuntimeCapabilityCompleted, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
        RuntimeCapabilityResumeRequest, RuntimeStatusRequest, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    };
    use ironclaw_threads::InMemorySessionThreadService;
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    use ironclaw_turns::{
        CancelRunRequest, CancelRunResponse, GetRunStateRequest, ResumeTurnRequest,
        ResumeTurnResponse, SubmitTurnRequest, SubmitTurnResponse, TurnCoordinator, TurnError,
        TurnRunId, TurnRunState, TurnScope,
    };
    use tokio::sync::Notify;

    use super::{
        GithubIssueWorkflowRuntimeDeps, HostRuntimeGithubIssueWorkflowCapabilityDispatcher,
        IronClawGithubIssueWorkflowPort, WorkflowStageResultSinkSlot, spawn_github_issue_workflow,
    };

    #[tokio::test]
    async fn composition_workspace_manager_fails_closed_without_real_backend() {
        let manager = super::UnconfiguredWorkflowWorkspaceManager;

        let error = manager
            .prepare_workspace(PrepareWorkflowWorkspaceRequest {
                tenant_id: TenantId::new("tenant-workspace").unwrap(),
                creator_user_id: UserId::new("user-workspace").unwrap(),
                agent_id: Some(AgentId::new("agent-workspace").unwrap()),
                project_id: Some(ProjectId::new("project-workspace").unwrap()),
                workflow_run_id: GithubIssueWorkflowRunId::from_trusted(
                    "workflow-run-workspace".to_string(),
                )
                .unwrap(),
                issue: GithubIssueRef {
                    owner: "nearai".to_string(),
                    repo: "ironclaw".to_string(),
                    number: 42,
                    node_id: Some("issue-node-42".to_string()),
                    url: "https://github.com/nearai/ironclaw/issues/42".to_string(),
                    default_branch: "main".to_string(),
                },
                base_branch: "main".to_string(),
                requested_at: Utc::now(),
            })
            .await
            .expect_err("composition must not advertise synthetic workspace success");

        assert!(matches!(
            error,
            GithubIssueWorkflowError::PolicyDenied { .. }
        ));
    }

    #[tokio::test]
    async fn host_runtime_github_issue_workflow_provider_dispatcher_selects_configured_account() {
        let host_runtime = Arc::new(RecordingHostRuntime::with_output(serde_json::json!([
            {
                "number": 42,
                "html_url": "https://github.com/nearai/ironclaw/issues/42",
                "updated_at": "2026-06-22T10:30:00Z"
            }
        ])));
        let port: Arc<dyn GithubIssueWorkflowPort> =
            Arc::new(IronClawGithubIssueWorkflowPort::new(Arc::new(
                HostRuntimeGithubIssueWorkflowCapabilityDispatcher::new(
                    host_runtime.clone(),
                    execution_context_for_test(),
                    trust_decision_for_test(),
                ),
            )));

        let hits = port
            .search_open_bug_issues(SearchGithubIssuesInput {
                provider_account_ref: provider_account("input-account"),
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                query: "repo:nearai/ironclaw is:issue state:open label:bug".to_string(),
                limit: 5,
            })
            .await
            .expect("production-shaped dispatch should invoke host runtime");

        assert_eq!(hits.len(), 1);
        let request = host_runtime
            .take_request()
            .expect("host runtime request should be captured");
        assert_eq!(request.capability_id.as_str(), "github.search_issues");
        assert_eq!(
            request.input,
            serde_json::json!({
                "query": "repo:nearai/ironclaw is:issue state:open label:bug",
                "limit": 5,
            })
        );
        assert_eq!(request.credential_account_selections.len(), 1);
        let selection = &request.credential_account_selections[0];
        assert_eq!(selection.provider.as_str(), "github");
        assert_eq!(selection.account_id.as_str(), "input-account");
    }

    #[tokio::test]
    async fn host_runtime_github_issue_workflow_provider_dispatcher_mints_invocation_per_call() {
        let host_runtime = Arc::new(RecordingHostRuntime::with_output(serde_json::json!([])));
        let port: Arc<dyn GithubIssueWorkflowPort> =
            Arc::new(IronClawGithubIssueWorkflowPort::new(Arc::new(
                HostRuntimeGithubIssueWorkflowCapabilityDispatcher::new(
                    host_runtime.clone(),
                    execution_context_for_test(),
                    trust_decision_for_test(),
                ),
            )));

        for _ in 0..2 {
            port.search_open_bug_issues(SearchGithubIssuesInput {
                provider_account_ref: provider_account("input-account"),
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                query: "repo:nearai/ironclaw is:issue state:open label:bug".to_string(),
                limit: 5,
            })
            .await
            .expect("production-shaped dispatch should invoke host runtime");
        }

        let requests = host_runtime.take_requests();
        assert_eq!(requests.len(), 2);
        assert_ne!(
            requests[0].context.invocation_id, requests[1].context.invocation_id,
            "background workflow capability dispatches must not reuse run-state ids"
        );
        assert_ne!(
            requests[0].context.resource_scope.invocation_id,
            requests[1].context.resource_scope.invocation_id,
            "resource scope invocation ids must stay aligned and fresh"
        );
    }

    #[tokio::test]
    async fn spawned_github_issue_workflow_dispatches_with_reborn_owner_scope() {
        let host_runtime = Arc::new(RecordingHostRuntime::with_output(serde_json::json!([])));
        let handle = spawn_github_issue_workflow(
            crate::runtime_input::GithubIssueWorkflowSettings::enabled_for_tests(),
            GithubIssueWorkflowRuntimeDeps {
                repository: Arc::new(
                    ironclaw_github_issue_workflow::InMemoryGithubIssueWorkflowRepository::default(
                    ),
                ),
                stage_result_sink_slot: Arc::new(WorkflowStageResultSinkSlot::new()),
                host_runtime: host_runtime.clone(),
                config_source: Arc::new(StaticWorkflowConfigSource {
                    configs: vec![GithubIssueWorkflowConfig {
                        tenant_id: TenantId::new("workflow-tenant").unwrap(),
                        project_id: ProjectId::new("workflow-project").unwrap(),
                        owner_user_id: UserId::new("workflow-owner").unwrap(),
                        repositories: vec![
                            GithubRepositorySelector::new("nearai", "ironclaw").unwrap(),
                        ],
                        candidate_selector: GithubIssueCandidateSelector::default(),
                        max_active_runs_per_repo: 1,
                        default_run_profile: "github-bug-workflow-v1".to_string(),
                        provider_account_ref: provider_account("runtime-account"),
                    }],
                }),
                project_access: Arc::new(AllowWorkflowProjectAccess),
                workspace_manager: super::test_only_unconfigured_workspace_manager(),
                thread_service: Arc::new(InMemorySessionThreadService::default()),
                turn_coordinator: Arc::new(PanicTurnCoordinator),
                tenant_id: TenantId::new("workflow-tenant").unwrap(),
                actor_user_id: UserId::new("workflow-owner").unwrap(),
                default_agent_id: AgentId::new("workflow-agent").unwrap(),
                default_project_id: Some(ProjectId::new("workflow-project").unwrap()),
            },
        )
        .expect("workflow spawn should be configured")
        .expect("enabled workflow should spawn");

        let request = host_runtime.wait_for_request().await;
        handle.shutdown(Duration::from_secs(1)).await;

        assert_eq!(request.context.tenant_id.as_str(), "workflow-tenant");
        assert_eq!(request.context.user_id.as_str(), "workflow-owner");
        assert_eq!(
            request
                .context
                .agent_id
                .as_ref()
                .map(|agent_id| agent_id.as_str()),
            Some("workflow-agent")
        );
        assert_eq!(
            request
                .context
                .project_id
                .as_ref()
                .map(|project_id| project_id.as_str()),
            Some("workflow-project")
        );
        assert_eq!(
            request.context.resource_scope.tenant_id.as_str(),
            "workflow-tenant"
        );
        assert_eq!(
            request.context.resource_scope.user_id.as_str(),
            "workflow-owner"
        );
        assert_eq!(
            request
                .context
                .resource_scope
                .agent_id
                .as_ref()
                .map(|agent_id| agent_id.as_str()),
            Some("workflow-agent")
        );
        assert_eq!(
            request
                .context
                .resource_scope
                .project_id
                .as_ref()
                .map(|project_id| project_id.as_str()),
            Some("workflow-project")
        );
        let search_grant = request
            .context
            .grants
            .grants
            .iter()
            .find(|grant| grant.capability.as_str() == "github.search_issues")
            .expect("workflow context should grant GitHub issue search");
        assert!(
            search_grant
                .constraints
                .allowed_effects
                .contains(&EffectKind::Network)
        );
        assert!(
            search_grant
                .constraints
                .allowed_effects
                .contains(&EffectKind::UseSecret)
        );
        assert_eq!(
            search_grant.constraints.network.allowed_targets,
            vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "api.github.com".to_string(),
                port: None,
            }]
        );
        assert!(search_grant.constraints.network.deny_private_ip_ranges);
        assert_eq!(
            search_grant.issued_by,
            ironclaw_approvals::persistent_approval_grant_issuer()
        );
    }

    #[derive(Debug)]
    struct RecordingHostRuntime {
        output: serde_json::Value,
        requests: Mutex<Vec<RuntimeCapabilityRequest>>,
        notify: Notify,
    }

    impl RecordingHostRuntime {
        fn with_output(output: serde_json::Value) -> Self {
            Self {
                output,
                requests: Mutex::new(Vec::new()),
                notify: Notify::new(),
            }
        }

        fn take_request(&self) -> Option<RuntimeCapabilityRequest> {
            self.requests.lock().expect("request mutex").pop()
        }

        fn take_requests(&self) -> Vec<RuntimeCapabilityRequest> {
            std::mem::take(&mut *self.requests.lock().expect("request mutex"))
        }

        async fn wait_for_request(&self) -> RuntimeCapabilityRequest {
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if let Some(request) = self.take_request() {
                        return request;
                    }
                    self.notify.notified().await;
                }
            })
            .await
            .expect("host runtime request should be captured")
        }
    }

    #[async_trait]
    impl HostRuntime for RecordingHostRuntime {
        async fn invoke_capability(
            &self,
            request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            self.requests
                .lock()
                .expect("request mutex")
                .push(request.clone());
            self.notify.notify_waiters();
            Ok(RuntimeCapabilityOutcome::Completed(Box::new(
                RuntimeCapabilityCompleted {
                    capability_id: request.capability_id,
                    output: self.output.clone(),
                    display_preview: None,
                    usage: ResourceUsage::default(),
                },
            )))
        }

        async fn resume_capability(
            &self,
            _request: RuntimeCapabilityResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            panic!("resume_capability should not be called in this test");
        }

        async fn auth_resume_capability(
            &self,
            _request: RuntimeCapabilityAuthResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            panic!("auth_resume_capability should not be called in this test");
        }

        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
            panic!("visible_capabilities should not be called in this test");
        }

        async fn cancel_work(
            &self,
            _request: CancelRuntimeWorkRequest,
        ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
            panic!("cancel_work should not be called in this test");
        }

        async fn runtime_status(
            &self,
            _request: RuntimeStatusRequest,
        ) -> Result<HostRuntimeStatus, HostRuntimeError> {
            panic!("runtime_status should not be called in this test");
        }

        async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
            panic!("health should not be called in this test");
        }
    }

    fn execution_context_for_test() -> ExecutionContext {
        ExecutionContext::local_default(
            UserId::new("workflow-user").unwrap(),
            ExtensionId::new("github").unwrap(),
            RuntimeKind::Wasm,
            TrustClass::UserTrusted,
            CapabilitySet::default(),
            MountView::default(),
        )
        .unwrap()
    }

    fn trust_decision_for_test() -> TrustDecision {
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: vec![EffectKind::DispatchCapability],
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::Default,
            evaluated_at: Utc::now(),
        }
    }

    fn provider_account(account_id: &str) -> GithubProviderAccountRef {
        GithubProviderAccountRef {
            provider: "github".to_string(),
            account_id: account_id.to_string(),
        }
    }

    struct StaticWorkflowConfigSource {
        configs: Vec<GithubIssueWorkflowConfig>,
    }

    #[async_trait]
    impl GithubIssueWorkflowConfigSource for StaticWorkflowConfigSource {
        async fn list_enabled_workflow_configs(
            &self,
        ) -> Result<Vec<GithubIssueWorkflowConfig>, GithubIssueWorkflowError> {
            Ok(self.configs.clone())
        }
    }

    struct AllowWorkflowProjectAccess;

    #[async_trait]
    impl WorkflowProjectAccess for AllowWorkflowProjectAccess {
        async fn assert_workflow_config_access(
            &self,
            _request: WorkflowConfigAccessRequest,
        ) -> Result<(), GithubIssueWorkflowError> {
            Ok(())
        }

        async fn assert_workflow_project_access(
            &self,
            _request: WorkflowProjectAccessRequest,
        ) -> Result<(), GithubIssueWorkflowError> {
            Ok(())
        }
    }

    struct PanicTurnCoordinator;

    #[async_trait]
    impl TurnCoordinator for PanicTurnCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            panic!("prepare_turn should not be called in this test")
        }

        async fn submit_turn(
            &self,
            _request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            panic!("submit_turn should not be called in this test")
        }

        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            panic!("resume_turn should not be called in this test")
        }

        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            panic!("cancel_run should not be called in this test")
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            panic!("get_run_state should not be called in this test")
        }
    }
}
