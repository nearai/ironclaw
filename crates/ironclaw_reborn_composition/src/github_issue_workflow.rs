use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_github_issue_workflow::{
    AcceptStageResultInput, AcceptStageResultOutcome, CreateDraftPullRequestInput,
    CreateIssueCommentInput, GetAuthenticatedWorkflowActorInput, GetGithubIssueInput,
    GetPullRequestInput, GithubActorSnapshot, GithubCheckConclusion, GithubCommentRef,
    GithubIssueCandidateSelector, GithubIssueCommentSnapshot, GithubIssueProviderSnapshot,
    GithubIssueSearchHit, GithubIssueStage, GithubIssueStageRunId, GithubIssueWorkflowConfig,
    GithubIssueWorkflowConfigSource, GithubIssueWorkflowError, GithubIssueWorkflowPoller,
    GithubIssueWorkflowPollerConfig, GithubIssueWorkflowPollerPorts, GithubIssueWorkflowPort,
    GithubIssueWorkflowRepository, GithubIssueWorkflowRunId, GithubIssueWorkspaceSession,
    GithubIssueWorkspaceSessionId, GithubProviderAccountRef, GithubPullRequestCheckSnapshot,
    GithubPullRequestRef, GithubPullRequestSnapshot, GithubRepositorySelector,
    GithubReviewCommentSnapshot, ListIssueCommentsInput, ListPullRequestChecksInput,
    ListPullRequestReviewCommentsInput, ListPullRequestsInput, PrepareWorkflowWorkspaceOutcome,
    PrepareWorkflowWorkspaceRequest, RecordWorkflowEventInput, SearchGithubIssuesInput,
    StageCompletedPayload, StageTurnSubmitter, SubmitStageTurnOutcome, SubmitStageTurnRequest,
    WorkflowActorScope, WorkflowClock, WorkflowConfigAccessRequest, WorkflowEventEnvelope,
    WorkflowEventSourceKind, WorkflowProjectAccess, WorkflowProjectAccessRequest, WorkflowWorkerId,
    WorkflowWorkspaceManager, WorkflowWorkspaceRef, issue_binding_ref, stage_result_reported_key,
    validate_stage_result,
};
use ironclaw_host_api::{
    AgentId, CapabilityId, CapabilitySet, ExecutionContext, ExtensionId, MountView, ProjectId,
    ResourceEstimate, RuntimeKind, TenantId, ThreadId, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult, ReportWorkflowStageResultInput,
    RuntimeFailureKind, WorkflowStageResultAck, WorkflowStageResultSink,
    WorkflowStageResultSinkError, builtin_first_party_handlers_with_workflow_stage_result_sink,
};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    SpawnSubagentFlavorDescriptor, SubagentDefinition, SubagentDefinitionResolver, SubagentKindId,
    build_spawn_subagent_parameters_schema,
};
use ironclaw_product_context::InboundClassification;
use ironclaw_product_workflow::{
    ProjectCaller, ProjectService, ProjectServiceError, RebornGetProjectRequest,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent, MessageStatus,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadService, ThreadMessageId,
    ThreadScope,
};
use ironclaw_trust::TrustDecision;
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustProvenance};
use ironclaw_turns::{
    AcceptedMessageRef, IdempotencyKey, ProductTurnContext, ReplyTargetBindingRef,
    RunOriginAdapter, RunProfileRequest, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
    TurnActor, TurnCoordinator, TurnError, TurnRunId, TurnScope, TurnSurfaceType,
    run_profile::{
        CapabilitySurfaceProfileId, InMemoryRunProfileRegistry, InMemoryRunProfileResolver,
        LoopRunContext, RunProfileDefinition, RunProfileRegistryError,
    },
};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

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
const PROJECT_METADATA_GITHUB_ISSUE_WORKFLOW_KEY: &str = "github_issue_workflow";
const DEFAULT_GITHUB_ISSUE_WORKFLOW_RUN_PROFILE: &str = "default";

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

pub(crate) fn stage_capability_profiles() -> &'static [GithubIssueWorkflowCapabilityProfile] {
    GITHUB_ISSUE_WORKFLOW_STAGE_PROFILES
}

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

fn is_github_issue_workflow_context(run_context: &LoopRunContext) -> bool {
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
        registry.register(RunProfileDefinition::interactive_like(
            profile_id,
            descriptor.clone(),
            checkpoint_schema_id.clone(),
            checkpoint_schema_version,
            capability_surface_profile_id,
        ))?;
    }
    Ok(())
}

fn invalid_run_profile(reason: String) -> RunProfileRegistryError {
    RunProfileRegistryError::InvalidProfile { reason }
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
        input: ReportWorkflowStageResultInput,
    ) -> Result<WorkflowStageResultAck, WorkflowStageResultSinkError> {
        let Some(sink) = self.inner.get().cloned() else {
            return Err(WorkflowStageResultSinkError::Unavailable);
        };
        sink.report_stage_result(input).await
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

pub(crate) struct GithubWorkflowStageResultSink {
    repository: Arc<dyn GithubIssueWorkflowRepository>,
}

impl GithubWorkflowStageResultSink {
    pub(crate) fn new(repository: Arc<dyn GithubIssueWorkflowRepository>) -> Self {
        Self { repository }
    }
}

#[async_trait]
impl WorkflowStageResultSink for GithubWorkflowStageResultSink {
    async fn report_stage_result(
        &self,
        input: ReportWorkflowStageResultInput,
    ) -> Result<WorkflowStageResultAck, WorkflowStageResultSinkError> {
        let workflow_run_id = GithubIssueWorkflowRunId::from_trusted(input.workflow_run_id)
            .map_err(stage_result_invalid_input)?;
        let stage_run_id = GithubIssueStageRunId::from_trusted(input.stage_run_id.clone())
            .map_err(stage_result_invalid_input)?;
        let _turn_run_id = TurnRunId::parse(&input.turn_run_id).map_err(|error| {
            WorkflowStageResultSinkError::InvalidInput {
                reason: format!("invalid turn_run_id: {error}"),
            }
        })?;
        let stage = serde_json::from_value::<GithubIssueStage>(JsonValue::String(input.stage))
            .map_err(|error| WorkflowStageResultSinkError::InvalidInput {
                reason: format!("invalid stage: {error}"),
            })?;
        if input.completion_nonce.trim().is_empty() {
            return Err(WorkflowStageResultSinkError::InvalidInput {
                reason: "completion_nonce must not be empty".to_string(),
            });
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
                Ok(WorkflowStageResultAck {
                    accepted: true,
                    duplicate: false,
                    stage_run_id: input.stage_run_id,
                })
            }
            AcceptStageResultOutcome::NotActiveStage { .. } => {
                Err(WorkflowStageResultSinkError::StageNotActive)
            }
            AcceptStageResultOutcome::Terminal => Err(WorkflowStageResultSinkError::StageNotActive),
        }
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
        GithubIssueWorkflowPolicy, GithubIssueWorkflowPolicyPorts, GithubIssueWorkflowRepository,
        GithubIssueWorkflowRun, GithubIssueWorkflowRunKey, GithubIssueWorkspaceSession,
        GithubIssueWorkspaceSessionId, GithubProviderAccountRef, GithubPullRequestRef,
        GithubPullRequestSnapshot, GithubRepositorySelector, InMemoryGithubIssueWorkflowRepository,
        ListIssueCommentsInput, ListPullRequestsInput, ListWorkflowEventsAfterInput,
        PrepareWorkflowWorkspaceOutcome, PrepareWorkflowWorkspaceRequest, StageTurnSubmitter,
        SubmitStageTurnOutcome, SubmitStageTurnRequest, TransitionOutcome, WorkflowClock,
        WorkflowEventSourceKind, WorkflowProjectAccess, WorkflowProjectAccessRequest,
        WorkflowRunTransition, WorkflowWorkerId, WorkflowWorkspaceManager,
        WorkflowWorkspaceMountRef, WorkflowWorkspaceRef,
    };
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
    use ironclaw_host_runtime::{ReportWorkflowStageResultInput, WorkflowStageResultSink};
    use ironclaw_turns::TurnRunId;
    use serde_json::json;
    use tokio::sync::Mutex;

    use super::GithubWorkflowStageResultSink;

    #[tokio::test]
    async fn stage_result_sink_accepts_result_and_records_stage_completed_event() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Triage).await;
        let sink = GithubWorkflowStageResultSink::new(repository.clone());

        let ack = sink
            .report_stage_result(ReportWorkflowStageResultInput {
                workflow_run_id: workflow_run_id.as_str().to_string(),
                stage_run_id: stage_run_id.as_str().to_string(),
                turn_run_id: TurnRunId::new().to_string(),
                stage: "triage".to_string(),
                schema_version: "triage.v1".to_string(),
                completion_nonce: "nonce-triage".to_string(),
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
            })
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
    async fn stage_result_sink_rejects_invalid_implementation_without_recording_event() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Implementation).await;
        let sink = GithubWorkflowStageResultSink::new(repository.clone());

        let error = sink
            .report_stage_result(ReportWorkflowStageResultInput {
                workflow_run_id: workflow_run_id.as_str().to_string(),
                stage_run_id: stage_run_id.as_str().to_string(),
                turn_run_id: TurnRunId::new().to_string(),
                stage: "implementation".to_string(),
                schema_version: "implementation.v1".to_string(),
                completion_nonce: "nonce-implementation".to_string(),
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
            })
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
        let sink = GithubWorkflowStageResultSink::new(repository.clone());
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
        let run = create_stage_run(&repository, run, GithubIssueStage::Implementation).await;
        report_stage_result(
            &sink,
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
        let run = match repository
            .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
                tenant_id: TenantId::new("tenant-stage-result-sink").unwrap(),
                creator_user_id: UserId::new("user-stage-result-sink").unwrap(),
                agent_id: Some(AgentId::new("agent-stage-result-sink").unwrap()),
                project_id: Some(ProjectId::new("project-stage-result-sink").unwrap()),
                provider_account_ref: None,
                issue_ref: issue(),
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
        run: &GithubIssueWorkflowRun,
        stage: GithubIssueStage,
        schema_version: &str,
        result: serde_json::Value,
    ) {
        let stage_run_id = run
            .active_stage_run_id
            .as_ref()
            .expect("active stage")
            .as_str()
            .to_string();
        let ack = sink
            .report_stage_result(ReportWorkflowStageResultInput {
                workflow_run_id: run.workflow_run_id.as_str().to_string(),
                stage_run_id,
                turn_run_id: TurnRunId::new().to_string(),
                stage: stage_name(&stage).to_string(),
                schema_version: schema_version.to_string(),
                completion_nonce: format!("nonce-{schema_version}"),
                result,
            })
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
                tracing::warn!(?error, "GitHub issue workflow poller task join failed");
            }
            Err(_) => {
                tracing::warn!(
                    ?timeout,
                    "GitHub issue workflow poller did not stop before shutdown timeout; aborting"
                );
                handle.abort();
                if let Err(error) = handle.await
                    && error.is_panic()
                {
                    tracing::warn!(?error, "aborted GitHub issue workflow poller task panicked");
                }
            }
        }
    }
}

pub(crate) struct GithubIssueWorkflowRuntimeDeps {
    pub(crate) repository: Arc<dyn GithubIssueWorkflowRepository>,
    pub(crate) stage_result_sink_slot: Arc<WorkflowStageResultSinkSlot>,
    pub(crate) host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime>,
    pub(crate) configured_provider_account_ref: GithubProviderAccountRef,
    pub(crate) config_source: Arc<dyn GithubIssueWorkflowConfigSource>,
    pub(crate) project_access: Arc<dyn WorkflowProjectAccess>,
    pub(crate) workspace_manager: Arc<dyn WorkflowWorkspaceManager>,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) turn_coordinator: Arc<dyn TurnCoordinator>,
    pub(crate) actor_user_id: UserId,
    pub(crate) default_agent_id: AgentId,
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
        configured_provider_account_ref,
        config_source,
        project_access,
        workspace_manager,
        thread_service,
        turn_coordinator,
        actor_user_id,
        default_agent_id,
    } = deps;
    let sink: Arc<dyn WorkflowStageResultSink> =
        Arc::new(GithubWorkflowStageResultSink::new(Arc::clone(&repository)));
    stage_result_sink_slot
        .set(sink)
        .map_err(|_| GithubIssueWorkflowError::InvalidConfig {
            reason: "workflow stage result sink slot was already initialized".to_string(),
        })?;

    let dispatcher = Arc::new(HostRuntimeGithubIssueWorkflowCapabilityDispatcher::new(
        host_runtime,
        workflow_execution_context(actor_user_id.clone())?,
        workflow_trust_decision(),
    ));
    let github_port = Arc::new(IronClawGithubIssueWorkflowPort::new(
        configured_provider_account_ref,
        dispatcher,
    ));
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
        },
        "github-bug-workflow-v1",
    );
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let poll_interval = settings.poll_interval;
    let handle = tokio::spawn(async move {
        run_github_issue_workflow_poller(poller, poll_interval, task_cancel).await;
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
                    blocked_configs = outcome.blocked_configs.len(),
                    blocked_runs = outcome.blocked_runs.len(),
                    "GitHub issue workflow poller tick completed"
                );
            }
            Err(error) => {
                tracing::warn!(?error, "GitHub issue workflow poller tick failed");
            }
        }
        if !sleep_or_cancel(poll_interval, &cancel).await {
            return;
        }
    }
}

async fn sleep_or_cancel(delay: Duration, cancel: &CancellationToken) -> bool {
    tokio::select! {
        _ = cancel.cancelled() => false,
        _ = tokio::time::sleep(delay) => true,
    }
}

fn workflow_execution_context(
    owner_user_id: UserId,
) -> Result<ExecutionContext, GithubIssueWorkflowError> {
    ExecutionContext::local_default(
        owner_user_id,
        ExtensionId::new("github_issue_workflow").map_err(workflow_invalid_config)?,
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        CapabilitySet::default(),
        MountView::default(),
    )
    .map_err(workflow_invalid_config)
}

fn workflow_trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![ironclaw_host_api::EffectKind::DispatchCapability],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
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

pub(crate) fn project_metadata_github_issue_workflow_config_source(
    project_service: Arc<dyn ProjectService>,
    tenant_id: TenantId,
    owner_user_id: UserId,
    project_id: ProjectId,
    configured_provider_account_ref: GithubProviderAccountRef,
) -> Arc<dyn GithubIssueWorkflowConfigSource> {
    Arc::new(ProjectMetadataGithubIssueWorkflowConfigSource {
        project_service,
        tenant_id,
        owner_user_id,
        project_id,
        configured_provider_account_ref,
    })
}

pub(crate) fn runtime_workflow_workspace_manager() -> Arc<dyn WorkflowWorkspaceManager> {
    Arc::new(RuntimeWorkflowWorkspaceManager)
}

struct SystemWorkflowClock;

impl WorkflowClock for SystemWorkflowClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

struct ProjectMetadataGithubIssueWorkflowConfigSource {
    project_service: Arc<dyn ProjectService>,
    tenant_id: TenantId,
    owner_user_id: UserId,
    project_id: ProjectId,
    configured_provider_account_ref: GithubProviderAccountRef,
}

#[async_trait]
impl GithubIssueWorkflowConfigSource for ProjectMetadataGithubIssueWorkflowConfigSource {
    async fn list_enabled_workflow_configs(
        &self,
    ) -> Result<Vec<GithubIssueWorkflowConfig>, GithubIssueWorkflowError> {
        let response = self
            .project_service
            .get_project(
                ProjectCaller {
                    tenant_id: self.tenant_id.clone(),
                    user_id: self.owner_user_id.clone(),
                },
                RebornGetProjectRequest {
                    project_id: self.project_id.to_string(),
                },
            )
            .await
            .map_err(project_service_error_to_workflow_error)?;

        let Some(section) = project_metadata_workflow_section(&response.project.metadata)? else {
            return Ok(Vec::new());
        };
        if !section.enabled {
            return Ok(Vec::new());
        }

        let repositories = section
            .repositories
            .unwrap_or_default()
            .into_iter()
            .map(|repository| GithubRepositorySelector::new(repository.owner, repository.repo))
            .collect::<Result<Vec<_>, _>>()?;
        let candidate_selector = match section.labels {
            Some(labels) => GithubIssueCandidateSelector { labels },
            None => GithubIssueCandidateSelector::default(),
        };
        let config = GithubIssueWorkflowConfig {
            tenant_id: self.tenant_id.clone(),
            project_id: self.project_id.clone(),
            owner_user_id: self.owner_user_id.clone(),
            repositories,
            candidate_selector,
            max_active_runs_per_repo: section.max_active_runs_per_repo.unwrap_or(1),
            default_run_profile: section
                .default_run_profile
                .unwrap_or_else(|| DEFAULT_GITHUB_ISSUE_WORKFLOW_RUN_PROFILE.to_string()),
            provider_account_ref: self.configured_provider_account_ref.clone(),
        };
        config.validate()?;
        Ok(vec![config])
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectMetadataGithubIssueWorkflowSection {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    repositories: Option<Vec<ProjectMetadataGithubRepositorySelector>>,
    #[serde(default)]
    labels: Option<Vec<String>>,
    #[serde(default)]
    max_active_runs_per_repo: Option<u32>,
    #[serde(default)]
    default_run_profile: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectMetadataGithubRepositorySelector {
    owner: String,
    repo: String,
}

fn project_metadata_workflow_section(
    metadata: &JsonValue,
) -> Result<Option<ProjectMetadataGithubIssueWorkflowSection>, GithubIssueWorkflowError> {
    let Some(metadata_object) = metadata.as_object() else {
        if metadata.is_null() {
            return Ok(None);
        }
        return Err(GithubIssueWorkflowError::InvalidConfig {
            reason: "project metadata must be an object or null".to_string(),
        });
    };
    let Some(section) = metadata_object.get(PROJECT_METADATA_GITHUB_ISSUE_WORKFLOW_KEY) else {
        return Ok(None);
    };
    if section.is_null() {
        return Ok(None);
    }
    serde_json::from_value(section.clone())
        .map(Some)
        .map_err(|error| GithubIssueWorkflowError::InvalidConfig {
            reason: format!(
                "project metadata `{PROJECT_METADATA_GITHUB_ISSUE_WORKFLOW_KEY}` is invalid: {error}"
            ),
        })
}

fn project_service_error_to_workflow_error(error: ProjectServiceError) -> GithubIssueWorkflowError {
    match error {
        ProjectServiceError::NotFound | ProjectServiceError::Denied => {
            GithubIssueWorkflowError::PolicyDenied {
                reason: "workflow project is not accessible".to_string(),
            }
        }
        ProjectServiceError::InvalidInput { field } => GithubIssueWorkflowError::InvalidConfig {
            reason: format!("workflow project reference is invalid: {field}"),
        },
        ProjectServiceError::Conflict => GithubIssueWorkflowError::Repository {
            reason: "workflow project service reported a conflict".to_string(),
        },
        ProjectServiceError::Unavailable => GithubIssueWorkflowError::Repository {
            reason: "workflow project service is unavailable".to_string(),
        },
        ProjectServiceError::Internal => GithubIssueWorkflowError::Repository {
            reason: "workflow project service returned an internal error".to_string(),
        },
    }
}

struct EmptyGithubIssueWorkflowConfigSource;

#[async_trait]
impl GithubIssueWorkflowConfigSource for EmptyGithubIssueWorkflowConfigSource {
    async fn list_enabled_workflow_configs(
        &self,
    ) -> Result<Vec<GithubIssueWorkflowConfig>, GithubIssueWorkflowError> {
        Ok(Vec::new())
    }
}

struct UnconfiguredWorkflowProjectAccess;

#[async_trait]
impl WorkflowProjectAccess for UnconfiguredWorkflowProjectAccess {
    async fn assert_workflow_config_access(
        &self,
        _request: WorkflowConfigAccessRequest,
    ) -> Result<(), GithubIssueWorkflowError> {
        Err(GithubIssueWorkflowError::PolicyDenied {
            reason: "GitHub issue workflow project access checker is not configured".to_string(),
        })
    }

    async fn assert_workflow_project_access(
        &self,
        _request: WorkflowProjectAccessRequest,
    ) -> Result<(), GithubIssueWorkflowError> {
        Err(GithubIssueWorkflowError::PolicyDenied {
            reason: "GitHub issue workflow project access checker is not configured".to_string(),
        })
    }
}

struct RuntimeWorkflowWorkspaceManager;

#[async_trait]
impl WorkflowWorkspaceManager for RuntimeWorkflowWorkspaceManager {
    async fn prepare_workspace(
        &self,
        request: PrepareWorkflowWorkspaceRequest,
    ) -> Result<PrepareWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
        let workspace_session_id = GithubIssueWorkspaceSessionId::new();
        let workspace_ref = WorkflowWorkspaceRef {
            thread_id: None,
            workspace_session_id: Some(workspace_session_id.clone()),
            turn_run_id: None,
        };
        let mount_ref = ironclaw_github_issue_workflow::WorkflowWorkspaceMountRef {
            mount_id: workspace_session_id.as_str().to_string(),
            alias: crate::local_dev_mounts::WORKSPACE_ALIAS.to_string(),
        };
        Ok(PrepareWorkflowWorkspaceOutcome {
            session: GithubIssueWorkspaceSession {
                workspace_session_id,
                workflow_run_id: request.workflow_run_id.clone(),
                repository: GithubRepositorySelector::new(
                    request.issue.owner.clone(),
                    request.issue.repo.clone(),
                )?,
                base_branch: request.base_branch,
                base_sha: None,
                working_branch: workflow_working_branch(&request.issue, &request.workflow_run_id),
                current_head_sha: None,
                workspace_ref,
                mount_ref,
                created_at: request.requested_at,
            },
        })
    }
}

fn workflow_working_branch(
    issue: &ironclaw_github_issue_workflow::GithubIssueRef,
    workflow_run_id: &GithubIssueWorkflowRunId,
) -> String {
    let owner = git_branch_component(&issue.owner);
    let repo = git_branch_component(&issue.repo);
    let short_run_id: String = workflow_run_id.as_str().chars().take(12).collect();
    format!(
        "ironclaw/github-bug/{owner}-{repo}-issue-{}-{short_run_id}",
        issue.number
    )
}

fn git_branch_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| match character {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' => character,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        "repo".to_string()
    } else {
        sanitized
    }
}

struct UnconfiguredWorkflowWorkspaceManager;

#[async_trait]
impl WorkflowWorkspaceManager for UnconfiguredWorkflowWorkspaceManager {
    async fn prepare_workspace(
        &self,
        _request: PrepareWorkflowWorkspaceRequest,
    ) -> Result<PrepareWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
        Err(GithubIssueWorkflowError::PolicyDenied {
            reason: "GitHub issue workflow workspace backend is not configured".to_string(),
        })
    }
}

#[cfg(test)]
mod project_metadata_github_issue_workflow_config_source_tests {
    use super::{
        ProjectMetadataGithubIssueWorkflowConfigSource, RuntimeWorkflowWorkspaceManager,
        git_branch_component,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_github_issue_workflow::{
        GithubIssueRef, GithubIssueWorkflowConfigSource, GithubIssueWorkflowError,
        GithubProviderAccountRef, PrepareWorkflowWorkspaceRequest, WorkflowWorkspaceManager,
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

    #[tokio::test]
    async fn project_metadata_config_source_builds_enabled_workflow_config() {
        let metadata = json!({
            "github_issue_workflow": {
                "enabled": true,
                "repositories": [
                    { "owner": "near", "repo": "ironclaw" }
                ],
                "labels": ["bug", "regression"],
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
    async fn runtime_workspace_manager_uses_virtual_workspace_mount_only() {
        let manager = RuntimeWorkflowWorkspaceManager;
        let outcome = manager
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
                    owner: "near".to_string(),
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
            .expect("workspace prepares");

        assert_eq!(outcome.session.mount_ref.alias, "/workspace");
        assert_eq!(
            outcome.session.workspace_ref.workspace_session_id.as_ref(),
            Some(&outcome.session.workspace_session_id)
        );
        assert!(outcome.session.workspace_ref.thread_id.is_none());
        assert!(outcome.session.workspace_ref.turn_run_id.is_none());
        assert!(
            outcome
                .session
                .working_branch
                .starts_with("ironclaw/github-bug/near-ironclaw-issue-42-workflow-run")
        );
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
        captured_get_project: Mutex<Option<(ProjectCaller, RebornGetProjectRequest)>>,
    }

    impl FakeProjectService {
        fn new(metadata: JsonValue) -> Self {
            Self {
                metadata,
                captured_get_project: Mutex::new(None),
            }
        }

        fn captured_get_project(&self) -> (ProjectCaller, RebornGetProjectRequest) {
            self.captured_get_project
                .lock()
                .expect("lock")
                .clone()
                .expect("get_project captured")
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
            *self.captured_get_project.lock().expect("lock") = Some((caller, request.clone()));
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubIssueWorkflowCapabilityDispatchRequest {
    pub capability_id: String,
    pub provider_account_ref: GithubProviderAccountRef,
    pub input: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubIssueWorkflowCapabilityDispatchError {
    AuthRequired,
    ApprovalRequired,
    Backend { kind: String, message: String },
}

#[async_trait]
pub trait GithubIssueWorkflowCapabilityDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        request: GithubIssueWorkflowCapabilityDispatchRequest,
    ) -> Result<JsonValue, GithubIssueWorkflowCapabilityDispatchError>;
}

#[allow(dead_code)]
pub(crate) struct HostRuntimeGithubIssueWorkflowCapabilityDispatcher {
    host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime>,
    execution_context: ExecutionContext,
    trust_decision: TrustDecision,
    estimate: ResourceEstimate,
}

#[allow(dead_code)]
impl HostRuntimeGithubIssueWorkflowCapabilityDispatcher {
    pub(crate) fn new(
        host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime>,
        execution_context: ExecutionContext,
        trust_decision: TrustDecision,
    ) -> Self {
        Self {
            host_runtime,
            execution_context,
            trust_decision,
            estimate: ResourceEstimate::default(),
        }
    }
}

#[async_trait]
impl GithubIssueWorkflowCapabilityDispatcher
    for HostRuntimeGithubIssueWorkflowCapabilityDispatcher
{
    async fn dispatch(
        &self,
        request: GithubIssueWorkflowCapabilityDispatchRequest,
    ) -> Result<JsonValue, GithubIssueWorkflowCapabilityDispatchError> {
        // The current host-runtime credential staging path selects product-auth
        // accounts by scope/provider/requester only; it has no account-id field
        // on RuntimeCapabilityRequest or RuntimeCredentialAccountRequest. Failing
        // closed here prevents silently dispatching under a different GitHub
        // account than the workflow explicitly selected.
        Err(GithubIssueWorkflowCapabilityDispatchError::Backend {
            kind: "provider_account_selection_unavailable".to_string(),
            message: format!(
                "host runtime cannot honor explicit GitHub provider account selection for {}",
                request.capability_id
            ),
        })
    }
}

pub(crate) struct IronClawGithubIssueWorkflowPort<D> {
    configured_provider_account_ref: GithubProviderAccountRef,
    dispatcher: Arc<D>,
}

impl<D> IronClawGithubIssueWorkflowPort<D> {
    pub(crate) fn new(
        configured_provider_account_ref: GithubProviderAccountRef,
        dispatcher: Arc<D>,
    ) -> Self {
        Self {
            configured_provider_account_ref,
            dispatcher,
        }
    }
}

#[async_trait]
impl<D> GithubIssueWorkflowPort for IronClawGithubIssueWorkflowPort<D>
where
    D: GithubIssueWorkflowCapabilityDispatcher,
{
    async fn search_open_bug_issues(
        &self,
        input: SearchGithubIssuesInput,
    ) -> Result<Vec<GithubIssueSearchHit>, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_SEARCH_ISSUES_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "query": input.query,
                    "limit": input.limit,
                }),
            )
            .await?;
        normalize_issue_search_hits(&response, &input.owner, &input.repo)
    }

    async fn get_issue(
        &self,
        input: GetGithubIssueInput,
    ) -> Result<GithubIssueProviderSnapshot, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_GET_ISSUE_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.owner,
                    "repo": input.repo,
                    "issue_number": input.number,
                }),
            )
            .await?;
        normalize_issue_snapshot(&response, &input.owner, &input.repo, input.number)
    }

    async fn get_authenticated_workflow_actor(
        &self,
        _input: GetAuthenticatedWorkflowActorInput,
    ) -> Result<GithubActorSnapshot, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_GET_AUTHENTICATED_USER_CAPABILITY_ID,
                self.configured_provider_account_ref.clone(),
                json!({}),
            )
            .await?;
        normalize_actor_snapshot(&response)
    }

    async fn list_issue_comments(
        &self,
        input: ListIssueCommentsInput,
    ) -> Result<Vec<GithubIssueCommentSnapshot>, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID,
                self.configured_provider_account_ref.clone(),
                json!({
                    "owner": input.issue.owner,
                    "repo": input.issue.repo,
                    "issue_number": input.issue.number,
                }),
            )
            .await?;
        normalize_issue_comments(&response, &input.issue)
    }

    async fn create_issue_comment(
        &self,
        input: CreateIssueCommentInput,
    ) -> Result<GithubCommentRef, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_COMMENT_ISSUE_CAPABILITY_ID,
                self.configured_provider_account_ref.clone(),
                json!({
                    "owner": input.issue.owner,
                    "repo": input.issue.repo,
                    "issue_number": input.issue.number,
                    "body": input.body,
                }),
            )
            .await?;
        normalize_comment_ref(
            &response,
            Some(&input.issue),
            GITHUB_COMMENT_ISSUE_CAPABILITY_ID,
        )
    }

    async fn list_pull_requests(
        &self,
        input: ListPullRequestsInput,
    ) -> Result<Vec<GithubPullRequestSnapshot>, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_LIST_PULL_REQUESTS_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.owner.clone(),
                    "repo": input.repo.clone(),
                    "state": input.state.clone(),
                    "page": 1,
                    "limit": input.limit,
                }),
            )
            .await?;
        normalize_pull_request_snapshots(&response, &input.owner, &input.repo)
    }

    async fn create_draft_pull_request(
        &self,
        input: CreateDraftPullRequestInput,
    ) -> Result<GithubPullRequestRef, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_CREATE_PULL_REQUEST_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.owner,
                    "repo": input.repo,
                    "title": input.title,
                    "head": input.head_branch,
                    "base": input.base_branch,
                    "body": input.body,
                    "draft": true,
                }),
            )
            .await?;
        normalize_pull_request_ref(&response, &input.owner, &input.repo)
    }

    async fn get_pull_request(
        &self,
        input: GetPullRequestInput,
    ) -> Result<GithubPullRequestSnapshot, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_GET_PULL_REQUEST_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.owner.clone(),
                    "repo": input.repo.clone(),
                    "pr_number": input.number,
                }),
            )
            .await?;
        normalize_pull_request_snapshot(
            &response,
            &input.owner,
            &input.repo,
            GITHUB_GET_PULL_REQUEST_CAPABILITY_ID,
        )
    }

    async fn list_pull_request_checks(
        &self,
        input: ListPullRequestChecksInput,
    ) -> Result<Vec<GithubPullRequestCheckSnapshot>, GithubIssueWorkflowError> {
        let head_ref =
            input
                .head_sha
                .clone()
                .ok_or_else(|| GithubIssueWorkflowError::ProviderRead {
                    reason: format!(
                        "GitHub pull request {}/{}#{} has no head SHA for status lookup",
                        input.owner, input.repo, input.pull_request_number
                    ),
                })?;
        let response = self
            .dispatch_capability(
                GITHUB_GET_COMBINED_STATUS_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.owner,
                    "repo": input.repo,
                    "ref": head_ref,
                }),
            )
            .await?;
        normalize_combined_status_checks(&response, input.head_sha.as_deref(), input.limit)
    }

    async fn list_pull_request_review_comments(
        &self,
        input: ListPullRequestReviewCommentsInput,
    ) -> Result<Vec<GithubReviewCommentSnapshot>, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.owner,
                    "repo": input.repo,
                    "pr_number": input.pull_request_number,
                    "page": 1,
                    "limit": input.limit,
                }),
            )
            .await?;
        normalize_review_comments(&response)
    }
}

impl<D> IronClawGithubIssueWorkflowPort<D>
where
    D: GithubIssueWorkflowCapabilityDispatcher,
{
    async fn dispatch_capability(
        &self,
        capability_id: &str,
        provider_account_ref: GithubProviderAccountRef,
        input: JsonValue,
    ) -> Result<JsonValue, GithubIssueWorkflowError> {
        self.dispatcher
            .dispatch(GithubIssueWorkflowCapabilityDispatchRequest {
                capability_id: capability_id.to_string(),
                provider_account_ref,
                input,
            })
            .await
            .map_err(|error| map_dispatch_error(capability_id, error))
    }
}

fn map_dispatch_error(
    capability_id: &str,
    error: GithubIssueWorkflowCapabilityDispatchError,
) -> GithubIssueWorkflowError {
    match error {
        GithubIssueWorkflowCapabilityDispatchError::AuthRequired => {
            GithubIssueWorkflowError::PolicyDenied {
                reason: format!("GitHub capability {capability_id} requires authentication"),
            }
        }
        GithubIssueWorkflowCapabilityDispatchError::ApprovalRequired => {
            GithubIssueWorkflowError::PolicyDenied {
                reason: format!("GitHub capability {capability_id} requires approval"),
            }
        }
        GithubIssueWorkflowCapabilityDispatchError::Backend { kind, .. } => {
            if kind == RuntimeFailureKind::Transient.as_str()
                || kind == RuntimeFailureKind::Resource.as_str()
            {
                GithubIssueWorkflowError::ProviderRateLimited {
                    reason: format!("GitHub capability {capability_id} failed ({kind})"),
                }
            } else {
                GithubIssueWorkflowError::ProviderRead {
                    reason: format!("GitHub capability {capability_id} failed ({kind})"),
                }
            }
        }
    }
}

fn normalize_issue_search_hits(
    value: &JsonValue,
    owner: &str,
    repo: &str,
) -> Result<Vec<GithubIssueSearchHit>, GithubIssueWorkflowError> {
    let items = match value {
        JsonValue::Array(items) => items,
        _ => required_array(value, &["items"], GITHUB_SEARCH_ISSUES_CAPABILITY_ID)?,
    };
    items
        .iter()
        .map(|item| {
            let number = required_u64(item, &["number"], GITHUB_SEARCH_ISSUES_CAPABILITY_ID)?;
            Ok(GithubIssueSearchHit {
                owner: owner.to_string(),
                repo: repo.to_string(),
                number,
                node_id: optional_string(item, &[&["node_id"]]),
                url: issue_like_url(item, owner, repo, number),
                default_branch: optional_string(
                    item,
                    &[
                        &["repository", "default_branch"],
                        &["base", "repo", "default_branch"],
                        &["default_branch"],
                    ],
                )
                .unwrap_or_default(),
                updated_at: optional_rfc3339_datetime(item, &[&["updated_at"]]),
            })
        })
        .collect()
}

fn normalize_issue_snapshot(
    value: &JsonValue,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<GithubIssueProviderSnapshot, GithubIssueWorkflowError> {
    Ok(GithubIssueProviderSnapshot {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
        node_id: optional_string(value, &[&["node_id"]]),
        url: issue_like_url(value, owner, repo, number),
        default_branch: optional_string(
            value,
            &[
                &["repository", "default_branch"],
                &["base", "repo", "default_branch"],
                &["default_branch"],
            ],
        )
        .unwrap_or_default(),
        title: required_string(value, &["title"], GITHUB_GET_ISSUE_CAPABILITY_ID)?.to_string(),
        body: optional_string(value, &[&["body"]]).unwrap_or_default(),
        state: required_string(value, &["state"], GITHUB_GET_ISSUE_CAPABILITY_ID)?.to_string(),
        labels: optional_labels(value),
        updated_at: optional_rfc3339_datetime(value, &[&["updated_at"]]),
    })
}

fn normalize_actor_snapshot(
    value: &JsonValue,
) -> Result<GithubActorSnapshot, GithubIssueWorkflowError> {
    Ok(GithubActorSnapshot {
        login: required_string(
            value,
            &["login"],
            GITHUB_GET_AUTHENTICATED_USER_CAPABILITY_ID,
        )?
        .to_string(),
        node_id: optional_string(value, &[&["node_id"]]),
    })
}

fn normalize_issue_comments(
    value: &JsonValue,
    issue: &ironclaw_github_issue_workflow::GithubIssueRef,
) -> Result<Vec<GithubIssueCommentSnapshot>, GithubIssueWorkflowError> {
    let comments = match value {
        JsonValue::Array(items) => items,
        _ => required_array(value, &["items"], GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID)?,
    };

    comments
        .iter()
        .map(|comment| {
            Ok(GithubIssueCommentSnapshot {
                comment: normalize_comment_ref(
                    comment,
                    Some(issue),
                    GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID,
                )?,
                body: optional_string(comment, &[&["body"]]).unwrap_or_default(),
                author_login: optional_string(comment, &[&["user", "login"], &["author", "login"]])
                    .unwrap_or_default(),
                created_at: required_datetime(
                    comment,
                    &[&["created_at"]],
                    GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID,
                )?,
                updated_at: required_datetime(
                    comment,
                    &[&["updated_at"], &["created_at"]],
                    GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID,
                )?,
            })
        })
        .collect()
}

fn normalize_comment_ref(
    value: &JsonValue,
    issue: Option<&ironclaw_github_issue_workflow::GithubIssueRef>,
    capability_id: &str,
) -> Result<GithubCommentRef, GithubIssueWorkflowError> {
    let url = if let Some(url) = optional_string(value, &[&["html_url"], &["url"]]) {
        url
    } else if let (Some(issue), Some(comment_id)) =
        (issue, value.get("id").and_then(JsonValue::as_u64))
    {
        format!("{}#issuecomment-{comment_id}", issue.url)
    } else if let Some(issue) = issue {
        issue.url.clone()
    } else {
        return Err(invalid_output(
            capability_id,
            "comment response is missing url",
        ));
    };

    Ok(GithubCommentRef {
        node_id: optional_string(value, &[&["node_id"]]),
        url,
    })
}

fn normalize_pull_request_snapshots(
    value: &JsonValue,
    owner: &str,
    repo: &str,
) -> Result<Vec<GithubPullRequestSnapshot>, GithubIssueWorkflowError> {
    let items = match value {
        JsonValue::Array(items) => items,
        _ => required_array(value, &["items"], GITHUB_LIST_PULL_REQUESTS_CAPABILITY_ID)?,
    };
    items
        .iter()
        .map(|item| {
            normalize_pull_request_snapshot(
                item,
                owner,
                repo,
                GITHUB_LIST_PULL_REQUESTS_CAPABILITY_ID,
            )
        })
        .collect()
}

fn normalize_pull_request_snapshot(
    value: &JsonValue,
    owner: &str,
    repo: &str,
    capability_id: &str,
) -> Result<GithubPullRequestSnapshot, GithubIssueWorkflowError> {
    Ok(GithubPullRequestSnapshot {
        pull_request: normalize_pull_request_ref_with_capability(
            value,
            owner,
            repo,
            capability_id,
        )?,
        title: optional_string(value, &[&["title"]]).unwrap_or_default(),
        body: optional_string(value, &[&["body"]]).unwrap_or_default(),
        state: optional_string(value, &[&["state"]]).unwrap_or_else(|| "unknown".to_string()),
        draft: optional_bool(value, &[&["draft"]]).unwrap_or(false),
        merged: optional_bool(value, &[&["merged"]])
            .or_else(|| value.get("merged_at").map(|merged_at| !merged_at.is_null()))
            .unwrap_or(false),
        updated_at: optional_rfc3339_datetime(value, &[&["updated_at"]]),
    })
}

fn normalize_pull_request_ref(
    value: &JsonValue,
    owner: &str,
    repo: &str,
) -> Result<GithubPullRequestRef, GithubIssueWorkflowError> {
    normalize_pull_request_ref_with_capability(
        value,
        owner,
        repo,
        GITHUB_CREATE_PULL_REQUEST_CAPABILITY_ID,
    )
}

fn normalize_pull_request_ref_with_capability(
    value: &JsonValue,
    owner: &str,
    repo: &str,
    capability_id: &str,
) -> Result<GithubPullRequestRef, GithubIssueWorkflowError> {
    let number = required_u64(value, &["number"], capability_id)?;
    let head_branch =
        optional_string(value, &[&["head", "ref"], &["head_branch"]]).ok_or_else(|| {
            invalid_output(capability_id, "pull request response is missing head.ref")
        })?;

    Ok(GithubPullRequestRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
        node_id: optional_string(value, &[&["node_id"]]),
        url: optional_string(value, &[&["html_url"], &["url"]])
            .unwrap_or_else(|| format!("https://github.com/{owner}/{repo}/pull/{number}")),
        head_branch,
        head_sha: optional_string(value, &[&["head", "sha"], &["head_sha"]]),
    })
}

fn normalize_combined_status_checks(
    value: &JsonValue,
    fallback_head_sha: Option<&str>,
    limit: usize,
) -> Result<Vec<GithubPullRequestCheckSnapshot>, GithubIssueWorkflowError> {
    let statuses = match value {
        JsonValue::Array(items) => items,
        _ => required_array(
            value,
            &["statuses"],
            GITHUB_GET_COMBINED_STATUS_CAPABILITY_ID,
        )?,
    };
    statuses
        .iter()
        .take(limit)
        .map(|status| {
            let suite_or_run_id = optional_u64(status, &[&["id"]])
                .map(|id| id.to_string())
                .or_else(|| optional_string(status, &[&["node_id"], &["context"]]))
                .ok_or_else(|| {
                    invalid_output(
                        GITHUB_GET_COMBINED_STATUS_CAPABILITY_ID,
                        "combined status item is missing id or context",
                    )
                })?;
            let head_sha = optional_string(status, &[&["sha"]])
                .or_else(|| fallback_head_sha.map(ToString::to_string))
                .ok_or_else(|| {
                    invalid_output(
                        GITHUB_GET_COMBINED_STATUS_CAPABILITY_ID,
                        "combined status item is missing sha",
                    )
                })?;
            let conclusion = optional_string(status, &[&["state"]])
                .map(|state| GithubCheckConclusion::from_provider(&state))
                .unwrap_or(GithubCheckConclusion::Unknown);
            Ok(GithubPullRequestCheckSnapshot {
                suite_or_run_id,
                name: optional_string(status, &[&["context"], &["name"]]).unwrap_or_default(),
                head_sha,
                conclusion,
                completed_at: optional_rfc3339_datetime(
                    status,
                    &[&["updated_at"], &["created_at"]],
                ),
                details_url: optional_string(status, &[&["target_url"], &["url"]]),
            })
        })
        .collect()
}

fn normalize_review_comments(
    value: &JsonValue,
) -> Result<Vec<GithubReviewCommentSnapshot>, GithubIssueWorkflowError> {
    let comments = match value {
        JsonValue::Array(items) => items,
        _ => required_array(
            value,
            &["items"],
            GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID,
        )?,
    };

    comments
        .iter()
        .map(|comment| {
            Ok(GithubReviewCommentSnapshot {
                comment: normalize_comment_ref(
                    comment,
                    None,
                    GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID,
                )?,
                body: optional_string(comment, &[&["body"]]).unwrap_or_default(),
                author_login: optional_string(comment, &[&["user", "login"], &["author", "login"]])
                    .unwrap_or_default(),
                created_at: required_datetime(
                    comment,
                    &[&["created_at"]],
                    GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID,
                )?,
                updated_at: required_datetime(
                    comment,
                    &[&["updated_at"], &["created_at"]],
                    GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID,
                )?,
            })
        })
        .collect()
}

fn issue_like_url(value: &JsonValue, owner: &str, repo: &str, number: u64) -> String {
    optional_string(value, &[&["html_url"], &["url"]])
        .unwrap_or_else(|| format!("https://github.com/{owner}/{repo}/issues/{number}"))
}

fn optional_labels(value: &JsonValue) -> Vec<String> {
    value
        .get("labels")
        .and_then(JsonValue::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|label| match label {
                    JsonValue::String(name) => Some(name.clone()),
                    JsonValue::Object(_) => label
                        .get("name")
                        .and_then(JsonValue::as_str)
                        .map(ToString::to_string),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn required_array<'a>(
    value: &'a JsonValue,
    path: &[&str],
    capability_id: &str,
) -> Result<&'a Vec<JsonValue>, GithubIssueWorkflowError> {
    json_at_path(value, path)
        .and_then(JsonValue::as_array)
        .ok_or_else(|| {
            invalid_output(
                capability_id,
                &format!("missing array `{}`", path.join(".")),
            )
        })
}

fn required_string<'a>(
    value: &'a JsonValue,
    path: &[&str],
    capability_id: &str,
) -> Result<&'a str, GithubIssueWorkflowError> {
    json_at_path(value, path)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            invalid_output(
                capability_id,
                &format!("missing string `{}`", path.join(".")),
            )
        })
}

fn required_u64(
    value: &JsonValue,
    path: &[&str],
    capability_id: &str,
) -> Result<u64, GithubIssueWorkflowError> {
    json_at_path(value, path)
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| {
            invalid_output(
                capability_id,
                &format!("missing integer `{}`", path.join(".")),
            )
        })
}

fn required_datetime(
    value: &JsonValue,
    paths: &[&[&str]],
    capability_id: &str,
) -> Result<DateTime<Utc>, GithubIssueWorkflowError> {
    optional_rfc3339_datetime(value, paths).ok_or_else(|| {
        invalid_output(
            capability_id,
            &format!("missing timestamp `{}`", paths[0].join(".")),
        )
    })
}

fn optional_rfc3339_datetime(value: &JsonValue, paths: &[&[&str]]) -> Option<DateTime<Utc>> {
    optional_string(value, paths).and_then(|timestamp| {
        DateTime::parse_from_rfc3339(&timestamp)
            .ok()
            .map(|parsed| parsed.with_timezone(&Utc))
    })
}

fn optional_bool(value: &JsonValue, paths: &[&[&str]]) -> Option<bool> {
    paths
        .iter()
        .find_map(|path| json_at_path(value, path).and_then(JsonValue::as_bool))
}

fn optional_u64(value: &JsonValue, paths: &[&[&str]]) -> Option<u64> {
    paths
        .iter()
        .find_map(|path| json_at_path(value, path).and_then(JsonValue::as_u64))
}

fn optional_string(value: &JsonValue, paths: &[&[&str]]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| json_at_path(value, path).and_then(JsonValue::as_str))
        .map(ToString::to_string)
}

fn json_at_path<'a>(value: &'a JsonValue, path: &[&str]) -> Option<&'a JsonValue> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn invalid_output(capability_id: &str, detail: &str) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::ProviderRead {
        reason: format!("GitHub capability {capability_id} returned invalid output: {detail}"),
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
        "kind": "github_issue_workflow_stage",
        "workflow_run_id": request.stage_turn_identity.workflow_run_id.as_str(),
        "stage_run_id": request.stage_turn_identity.stage_run_id.as_str(),
        "stage": stage_label(&request.stage_turn_identity.stage),
        "attempt": request.stage_turn_identity.attempt,
        "workflow_policy_version": request.stage_turn_identity.workflow_policy_version.as_str(),
        "prompt_ref": request.prompt.content_ref.prompt_ref.as_str(),
        "prompt_version": request.prompt.content_ref.prompt_version.as_str(),
        "input_snapshot_hash": request.prompt.content_ref.input_snapshot_hash.as_str(),
        "prompt_content_hash": request.prompt.content_hash.as_str(),
    }))
    .map_err(|error| GithubIssueWorkflowError::Policy {
        reason: format!("failed to serialize stage thread metadata: {error}"),
    })
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
    use std::sync::Arc;

    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_github_issue_workflow::{
        GithubIssueRef, GithubIssueWorkflowError, GithubIssueWorkflowPort,
        GithubIssueWorkflowRunId, GithubProviderAccountRef, PrepareWorkflowWorkspaceRequest,
        SearchGithubIssuesInput, WorkflowWorkspaceManager,
    };
    use ironclaw_host_api::{
        AgentId, CapabilitySet, EffectKind, ExecutionContext, ExtensionId, MountView, ProjectId,
        RuntimeKind, TenantId, TrustClass, UserId,
    };
    use ironclaw_host_runtime::{
        CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, HostRuntime, HostRuntimeError,
        HostRuntimeHealth, HostRuntimeStatus, RuntimeCapabilityAuthResumeRequest,
        RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest,
        RuntimeStatusRequest, VisibleCapabilityRequest, VisibleCapabilitySurface,
    };
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};

    use super::{
        HostRuntimeGithubIssueWorkflowCapabilityDispatcher, IronClawGithubIssueWorkflowPort,
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
    async fn host_runtime_github_issue_workflow_provider_dispatcher_fails_closed_for_account_selection()
     {
        let port: Arc<dyn GithubIssueWorkflowPort> =
            Arc::new(IronClawGithubIssueWorkflowPort::new(
                provider_account("configured-account"),
                Arc::new(HostRuntimeGithubIssueWorkflowCapabilityDispatcher::new(
                    Arc::new(PanickingHostRuntime),
                    execution_context_for_test(),
                    trust_decision_for_test(),
                )),
            ));

        let error = port
            .search_open_bug_issues(SearchGithubIssuesInput {
                provider_account_ref: provider_account("input-account"),
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                query: "repo:nearai/ironclaw is:issue state:open label:bug".to_string(),
                limit: 5,
            })
            .await
            .expect_err("production-shaped dispatch should fail closed");

        assert!(matches!(
            error,
            GithubIssueWorkflowError::ProviderRead { .. }
        ));
        let rendered = error.to_string();
        assert!(rendered.contains("github.search_issues"));
        assert!(rendered.contains("provider_account_selection_unavailable"));
    }

    #[derive(Debug)]
    struct PanickingHostRuntime;

    #[async_trait]
    impl HostRuntime for PanickingHostRuntime {
        async fn invoke_capability(
            &self,
            _request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            panic!("host runtime should not be invoked when account selection cannot be honored");
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
}
