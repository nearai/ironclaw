use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_github_issue_workflow::{
    GithubIssueStage, GithubIssueWorkflowError, StageTurnSubmitter, SubmitStageTurnOutcome,
    SubmitStageTurnRequest, WorkflowActorScope,
};
use ironclaw_host_api::{AgentId, CapabilityId, ThreadId, UserId};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult, ReportWorkflowStageResultInput,
    WorkflowStageResultAck, WorkflowStageResultSink, WorkflowStageResultSinkError,
    builtin_first_party_handlers_with_workflow_stage_result_sink,
};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    SpawnSubagentFlavorDescriptor, SubagentDefinition, SubagentDefinitionResolver, SubagentKindId,
    build_spawn_subagent_parameters_schema,
};
use ironclaw_product_context::InboundClassification;
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent, MessageStatus,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadService, ThreadMessageId,
    ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, IdempotencyKey, ProductTurnContext, ReplyTargetBindingRef,
    RunOriginAdapter, RunProfileRequest, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
    TurnActor, TurnCoordinator, TurnError, TurnRunId, TurnScope, TurnSurfaceType,
    run_profile::{
        CapabilitySurfaceProfileId, InMemoryRunProfileRegistry, InMemoryRunProfileResolver,
        LoopRunContext, RunProfileDefinition, RunProfileRegistryError,
    },
};
use serde_json::json;

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
) -> Result<(), ironclaw_host_api::HostApiError> {
    let capability_id = CapabilityId::new(RESULT_SINK_CAPABILITY_ID)?;
    let workflow_registry = builtin_first_party_handlers_with_workflow_stage_result_sink(
        trigger_repository,
        Arc::new(UnavailableWorkflowStageResultSink),
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

struct UnavailableWorkflowStageResultSink;

#[async_trait]
impl WorkflowStageResultSink for UnavailableWorkflowStageResultSink {
    async fn report_stage_result(
        &self,
        _input: ReportWorkflowStageResultInput,
    ) -> Result<WorkflowStageResultAck, WorkflowStageResultSinkError> {
        Err(WorkflowStageResultSinkError::Unavailable)
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
