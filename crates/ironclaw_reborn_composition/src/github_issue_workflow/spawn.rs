//! Composition root for the GitHub issue workflow background poller.
//!
//! [`spawn_github_issue_workflow`] wires the repository, host-runtime capability
//! dispatcher, GitHub port, stage-turn submitter, stage-result sink, and poller
//! ports together, installs the stage-result sink into its deferred-init slot,
//! and launches the wake-driven poll loop on a tokio task. The returned
//! [`GithubIssueWorkflowRuntimeHandle`] owns cancellation and graceful shutdown.
//! Shared consts and the `workflow_invalid_config` error helper live in the
//! parent module and are referenced here via `super::`.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use ironclaw_github_issue_workflow::{
    GithubIssueWorkflowConfigSource, GithubIssueWorkflowError, GithubIssueWorkflowPoller,
    GithubIssueWorkflowPollerConfig, GithubIssueWorkflowPollerPorts,
    GithubIssueWorkflowPollerWakeReceiver, GithubIssueWorkflowPort, GithubIssueWorkflowRepository,
    GithubProviderAccountRef, StageTurnSubmitter, WorkflowClock, WorkflowProjectAccess,
    WorkflowWorkerId, WorkflowWorkspaceManager,
};
use ironclaw_host_api::{
    AgentId, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, CorrelationId,
    EffectKind, ExecutionContext, ExtensionId, GrantConstraints, InvocationId, MountView,
    NetworkPolicy, NetworkScheme, NetworkTargetPattern, Principal, ProjectId, ResourceScope,
    RuntimeKind, TenantId, TrustClass, UserId,
};
use ironclaw_host_runtime::WorkflowStageResultSink;
use ironclaw_threads::SessionThreadService;
use ironclaw_trust::TrustDecision;
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustProvenance};
use ironclaw_turns::TurnCoordinator;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::capability_dispatcher::HostRuntimeGithubIssueWorkflowCapabilityDispatcher;
use super::config_source::{
    EmptyGithubIssueWorkflowConfigSource, UnconfiguredWorkflowProjectAccess,
};
use super::stage_result_sink::{GithubWorkflowStageResultSink, WorkflowStageResultSinkSlot};
use super::stage_turn_submitter::IronClawStageTurnSubmitter;
use super::workspace_manager::UnconfiguredWorkflowWorkspaceManager;
use super::{
    GITHUB_COMMENT_ISSUE_CAPABILITY_ID, GITHUB_CREATE_PULL_REQUEST_CAPABILITY_ID,
    IronClawGithubIssueWorkflowPort, WORKFLOW_ADAPTER_ID, WORKFLOW_GITHUB_CAPABILITY_IDS,
    workflow_invalid_config,
};

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

#[cfg(test)]
mod project_metadata_github_issue_workflow_config_source_tests {
    use super::{IronClawGithubIssueWorkflowPollerPorts, test_only_unconfigured_workspace_manager};
    use crate::github_issue_workflow::config_source::{
        ProjectMetadataGithubIssueWorkflowConfigSource, ProjectServiceWorkflowProjectAccess,
    };
    use crate::github_issue_workflow::git_host::WorkflowGitRemoteConfig;
    use crate::github_issue_workflow::workflow_stage_workspace_mount_view_from_thread_metadata;
    use crate::github_issue_workflow::workspace_manager::{
        RuntimeWorkflowWorkspaceManager, git_branch_component, workflow_workspace_host_path,
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
