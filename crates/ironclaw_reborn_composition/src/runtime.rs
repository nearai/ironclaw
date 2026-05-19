//! Assembled Reborn runtime: substrate + drivers + worker, started as one.
//!
//! This module is the "later slice" the crate-level docstring promises:
//! product-level wiring on top of the substrate facades exposed by
//! `build_reborn_services`. It is the **only** place in the workspace where
//! `ironclaw_reborn` (drivers, host factory, model gateway bridge),
//! `ironclaw_threads` (session thread service), and (under the
//! `root-llm-provider` feature) `ironclaw_llm` are composed into a running
//! agent.
//!
//! Downstream callers (the CLI, future channel adapters, e2e harnesses) reach
//! this assembly only through:
//!
//! - [`build_reborn_runtime`] — construct + start the runtime
//! - [`RebornRuntime`] — task-level handle (`new_conversation`,
//!   `send_user_message`, `shutdown`)
//!
//! They never name the underlying `TurnCoordinator`, `SessionThreadService`,
//! `LoopExitApplier`, `HostManagedModelGateway`, etc. directly. That is the
//! property that satisfies the "narrow Reborn public surface" requirement
//! pinned by `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    EmptyLoopCapabilityPort, HostIdentityContextBuildError, HostIdentityContextCandidate,
    HostIdentityContextSource,
};
use ironclaw_reborn::loop_driver_host::LoopCapabilityPortFactory;
use ironclaw_reborn::loop_exit_applier::ThreadCheckpointLoopExitEvidencePort;
use ironclaw_reborn::runtime::{
    DefaultPlannedRuntimeBuildError, DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts,
    build_default_planned_runtime,
};
use ironclaw_reborn::turn_runner::{TurnRunnerWakeSender, TurnRunnerWorkerConfig};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    MessageKind, MessageStatus, SessionThreadService, ThreadHistoryRequest, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, GetRunStateRequest, IdempotencyKey,
    InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore, InMemoryTurnStateStore,
    ReplyTargetBindingRef, RunProfileResolutionRequest, SanitizedCancelReason, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnRunId,
    TurnScope, TurnStatus,
    run_profile::{
        AgentLoopHostError, InMemoryLoopHostMilestoneSink, LoopCapabilityPort, LoopRunContext,
        PromptMode,
    },
};

use crate::runtime_input::{PollSettings, RebornRuntimeIdentity, RebornRuntimeInput};
use crate::{RebornBuildError, RebornCompositionProfile, RebornServices, build_reborn_services};

#[cfg(feature = "root-llm-provider")]
use crate::runtime_input::RebornLlmConfig;

/// Stable identifier for a Reborn CLI conversation. Wraps a `ThreadId`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConversationId(pub ThreadId);

/// Final-form assistant reply read back from the session thread service after
/// a `send_user_message` completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantReply {
    pub conversation: ConversationId,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub text: Option<String>,
}

impl AssistantReply {
    /// True when a caller can treat the reply as a successful single-shot
    /// response. Recovery/failed/cancelled runs may still produce diagnostics,
    /// but they did not produce the requested assistant text.
    pub fn is_successful_final_reply(&self) -> bool {
        self.status == TurnStatus::Completed && self.text.is_some()
    }
}

/// Errors returned by `RebornRuntime` methods.
#[derive(Debug, Error)]
pub enum RebornRuntimeError {
    #[error("reborn runtime build failed: {0}")]
    Build(#[from] RebornBuildError),
    #[error("turn coordinator unavailable for assembled runtime")]
    TurnCoordinatorUnavailable,
    #[error("host runtime unavailable for assembled runtime")]
    HostRuntimeUnavailable,
    #[error("turn submission failed: {0}")]
    TurnSubmission(String),
    #[error("turn submission rejected: {reason}")]
    TurnRejected { reason: String },
    #[error("session thread service error: {0}")]
    ThreadService(String),
    #[error("turn coordinator error: {0}")]
    TurnCoordinator(String),
    #[error("run did not reach a terminal state within {timeout:?}")]
    RunTimeout { timeout: Duration },
    #[error("run cancelled by caller")]
    OperationCancelled,
    #[error("invalid scope or identifier: {reason}")]
    InvalidArgument { reason: String },
    #[cfg(feature = "root-llm-provider")]
    #[error("llm provider construction failed: {0}")]
    LlmProvider(String),
    #[error("turn-runner worker is no longer running")]
    WorkerStopped,
}

impl From<TurnError> for RebornRuntimeError {
    fn from(value: TurnError) -> Self {
        Self::TurnCoordinator(value.to_string())
    }
}

impl From<DefaultPlannedRuntimeBuildError> for RebornRuntimeError {
    fn from(value: DefaultPlannedRuntimeBuildError) -> Self {
        Self::InvalidArgument {
            reason: value.to_string(),
        }
    }
}

/// Started, running Reborn agent runtime.
///
/// `RebornRuntime` is the single user-facing handle returned by
/// [`build_reborn_runtime`]. Downstream code never reaches into the substrate
/// or worker machinery: it talks to the runtime through task-level methods.
pub struct RebornRuntime {
    services: RebornServices,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    thread_service: Arc<InMemorySessionThreadService>,
    thread_scope: ThreadScope,
    worker_handle: JoinHandle<()>,
    worker_cancel: CancellationToken,
    poll_settings: PollSettings,
    actor_user_id: UserId,
    source_binding_ref: SourceBindingRef,
    reply_target_binding_ref: ReplyTargetBindingRef,
    default_run_profile_id: String,
    wake_sender: TurnRunnerWakeSender,
    send_locks: Mutex<HashMap<ConversationId, Arc<Mutex<()>>>>,
}

impl RebornRuntime {
    /// Snapshot of the substrate facades produced by `build_reborn_services`.
    /// Exposed for diagnostics / readiness reporting; **not** for traffic.
    pub fn services(&self) -> &RebornServices {
        &self.services
    }

    /// Diagnostic id for the no-profile run profile selected by this runtime.
    pub fn default_run_profile_id(&self) -> &str {
        &self.default_run_profile_id
    }

    /// Create a fresh conversation. Returns the opaque conversation id used
    /// in subsequent `send_user_message` calls.
    ///
    /// The thread is materialized inside the session thread service so
    /// `accept_inbound_message` does not error on the first send.
    pub async fn new_conversation(&self) -> Result<ConversationId, RebornRuntimeError> {
        let thread_id =
            ThreadId::new(format!("reborn-conv-{}", Uuid::new_v4())).map_err(|reason| {
                RebornRuntimeError::InvalidArgument {
                    reason: reason.to_string(),
                }
            })?;
        self.thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: self.thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: self.actor_user_id.as_str().to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .map_err(|error| RebornRuntimeError::ThreadService(error.to_string()))?;
        Ok(ConversationId(thread_id))
    }

    /// Submit a user message into the conversation, wait for the run to
    /// reach a terminal state, and return the assistant reply read back
    /// from the session thread service.
    ///
    /// Without an LLM gateway wired in (i.e. when this crate is built
    /// without the `root-llm-provider` feature or `RebornLlmConfig` is not
    /// provided), the run will fail and the returned reply will surface
    /// that failure via `status = Failed` and `text = None`.
    pub async fn send_user_message(
        &self,
        conversation: &ConversationId,
        text: &str,
    ) -> Result<AssistantReply, RebornRuntimeError> {
        self.send_user_message_with_cancellation(conversation, text, CancellationToken::new())
            .await
    }

    /// Submit a user message with a cooperative cancellation token. If the
    /// token fires while waiting for completion, the runtime cancels the run
    /// before returning.
    pub async fn send_user_message_with_cancellation(
        &self,
        conversation: &ConversationId,
        text: &str,
        cancellation: CancellationToken,
    ) -> Result<AssistantReply, RebornRuntimeError> {
        let send_lock = self.send_lock_for(conversation).await;
        let _send_guard = send_lock.lock().await;
        if self.worker_handle.is_finished() {
            return Err(RebornRuntimeError::WorkerStopped);
        }
        let scope = self.turn_scope_for(&conversation.0);
        let accepted = self
            .thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: self.thread_scope.clone(),
                thread_id: conversation.0.clone(),
                actor_id: self.actor_user_id.as_str().to_string(),
                source_binding_id: Some(self.source_binding_ref.as_str().to_string()),
                reply_target_binding_id: Some(self.reply_target_binding_ref.as_str().to_string()),
                // This task-level API does not receive an upstream stable
                // event id, so mint a best-effort unique id scoped to the
                // caller-provided source binding.
                external_event_id: Some(format!(
                    "{}:{}",
                    self.source_binding_ref.as_str(),
                    Uuid::new_v4()
                )),
                content: MessageContent::text(text.to_string()),
            })
            .await
            .map_err(|error| RebornRuntimeError::ThreadService(error.to_string()))?;

        let accepted_message_ref = AcceptedMessageRef::new(format!("msg:{}", accepted.message_id))
            .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?;
        let idempotency_key = IdempotencyKey::new(format!(
            "{}-{}",
            self.source_binding_ref.as_str(),
            Uuid::new_v4()
        ))
        .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?;

        let response = self
            .turn_coordinator
            .submit_turn(SubmitTurnRequest {
                scope: scope.clone(),
                actor: TurnActor::new(self.actor_user_id.clone()),
                accepted_message_ref,
                source_binding_ref: self.source_binding_ref.clone(),
                reply_target_binding_ref: self.reply_target_binding_ref.clone(),
                requested_run_profile: None,
                idempotency_key,
                received_at: Utc::now(),
            })
            .await?;

        let SubmitTurnResponse::Accepted { run_id, .. } = response;
        if cancellation.is_cancelled() {
            self.cancel_run(
                &scope,
                run_id,
                SanitizedCancelReason::UserRequested,
                "caller-cancel",
            )
            .await?;
            return Err(RebornRuntimeError::OperationCancelled);
        }
        self.wake_sender.wake();

        let terminal_status = self
            .wait_for_terminal(&scope, run_id, &cancellation)
            .await?;
        let assistant_text = self
            .read_latest_assistant_text(&conversation.0, run_id)
            .await?;

        Ok(AssistantReply {
            conversation: conversation.clone(),
            run_id,
            status: terminal_status,
            text: assistant_text,
        })
    }

    /// Stop the turn-runner worker. Awaits the worker task to finish before
    /// returning.
    pub async fn shutdown(self) -> Result<(), RebornRuntimeError> {
        self.worker_cancel.cancel();
        if let Err(error) = self.worker_handle.await {
            if error.is_panic() {
                tracing::error!(%error, "reborn worker task panicked during shutdown");
            } else {
                tracing::warn!(%error, "reborn worker task was cancelled during shutdown");
            }
        }
        Ok(())
    }

    fn turn_scope_for(&self, thread_id: &ThreadId) -> TurnScope {
        TurnScope::new(
            self.thread_scope.tenant_id.clone(),
            Some(self.thread_scope.agent_id.clone()),
            self.thread_scope.project_id.clone(),
            thread_id.clone(),
        )
    }

    async fn send_lock_for(&self, conversation: &ConversationId) -> Arc<Mutex<()>> {
        let mut locks = self.send_locks.lock().await;
        Arc::clone(
            locks
                .entry(conversation.clone())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    async fn wait_for_terminal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        cancellation: &CancellationToken,
    ) -> Result<TurnStatus, RebornRuntimeError> {
        let start = std::time::Instant::now();
        loop {
            if self.worker_handle.is_finished() {
                return Err(RebornRuntimeError::WorkerStopped);
            }
            let state = self
                .turn_coordinator
                .get_run_state(GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await?;
            if state.status.is_terminal() {
                return Ok(state.status);
            }
            if state.status == TurnStatus::RecoveryRequired {
                // RecoveryRequired keeps the durable turn active because a
                // future recovery worker may resume it. The standalone
                // runtime has no recovery worker, so cancel it before
                // returning to release the conversation lock.
                let response = self
                    .cancel_run(
                        scope,
                        run_id,
                        SanitizedCancelReason::OperatorRequested,
                        "recovery-required-cancel",
                    )
                    .await?;
                return Ok(response.status);
            }
            if start.elapsed() > self.poll_settings.max_total {
                self.cancel_run(
                    scope,
                    run_id,
                    SanitizedCancelReason::Timeout,
                    "timeout-cancel",
                )
                .await?;
                return Err(RebornRuntimeError::RunTimeout {
                    timeout: self.poll_settings.max_total,
                });
            }
            tokio::select! {
                _ = cancellation.cancelled() => {
                    self.cancel_run(
                        scope,
                        run_id,
                        SanitizedCancelReason::UserRequested,
                        "caller-cancel",
                    )
                    .await?;
                    return Err(RebornRuntimeError::OperationCancelled);
                }
                _ = tokio::time::sleep(self.poll_settings.interval) => {}
            }
        }
    }

    async fn cancel_run(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        reason: SanitizedCancelReason,
        idempotency_suffix: &str,
    ) -> Result<CancelRunResponse, RebornRuntimeError> {
        let response = self
            .turn_coordinator
            .cancel_run(CancelRunRequest {
                scope: scope.clone(),
                actor: TurnActor::new(self.actor_user_id.clone()),
                run_id,
                reason,
                idempotency_key: IdempotencyKey::new(format!(
                    "{}-{}-{}",
                    self.source_binding_ref.as_str(),
                    idempotency_suffix,
                    run_id
                ))
                .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?,
            })
            .await?;
        self.wake_sender.wake();
        Ok(response)
    }

    async fn read_latest_assistant_text(
        &self,
        thread_id: &ThreadId,
        run_id: TurnRunId,
    ) -> Result<Option<String>, RebornRuntimeError> {
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: self.thread_scope.clone(),
                thread_id: thread_id.clone(),
            })
            .await
            .map_err(|error| RebornRuntimeError::ThreadService(error.to_string()))?;
        let run_id_str = run_id.to_string();
        let reply = history
            .messages
            .into_iter()
            .rev()
            .find(|message| {
                matches!(message.kind, MessageKind::Assistant)
                    && matches!(message.status, MessageStatus::Finalized)
                    && message.turn_run_id.as_deref() == Some(run_id_str.as_str())
            })
            .and_then(|message| message.content);
        Ok(reply)
    }
}

/// Build and start a Reborn agent runtime.
///
/// On return, the turn-runner worker is already running in the background and
/// the returned `RebornRuntime` is ready to accept `send_user_message` calls.
///
/// **Currently supported profiles:** only `RebornCompositionProfile::LocalDev`
/// is wired end-to-end here; production profiles will follow in a later slice
/// (they currently return their substrate-only `RebornServices` and need
/// durable thread/checkpoint stores wired before being driven). Passing a
/// production profile returns a "not yet wired" error rather than partially
/// starting an agent.
pub async fn build_reborn_runtime(
    input: RebornRuntimeInput,
) -> Result<RebornRuntime, RebornRuntimeError> {
    let RebornRuntimeInput {
        services: services_input,
        #[cfg(feature = "root-llm-provider")]
        llm,
        runner,
        poll,
        identity,
        #[cfg(test)]
        model_gateway_override,
    } = input;

    let services_input = services_input.ok_or(RebornRuntimeError::InvalidArgument {
        reason: "RebornRuntimeInput.services is required".to_string(),
    })?;

    let profile = services_input.profile();
    if !matches!(profile, RebornCompositionProfile::LocalDev) {
        return Err(RebornRuntimeError::InvalidArgument {
            reason: format!(
                "profile={profile} is not yet wired end-to-end by build_reborn_runtime; \
                 only local-dev is supported in this slice"
            ),
        });
    }

    let owner_id = services_input.owner_id().to_string();
    let services = build_reborn_services(services_input).await?;

    // For local-dev, we synthesize substrate handles the composition root
    // owns directly. These intentionally do not flow out of the runtime
    // facade — they're an implementation detail of how the runtime stitches
    // the worker to the thread service.
    let turn_state_store = Arc::new(InMemoryTurnStateStore::default());
    let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
    let loop_checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
    let thread_service = Arc::new(InMemorySessionThreadService::default());

    let validated_identity = validate_runtime_identity(identity)?;

    let tenant_id = validated_identity.tenant_id.clone();
    let agent_id = validated_identity.agent_id.clone();
    let actor_user_id =
        UserId::new(owner_id.clone()).map_err(|reason| RebornRuntimeError::InvalidArgument {
            reason: format!("user id: {reason}"),
        })?;
    let thread_scope = ThreadScope {
        tenant_id,
        agent_id,
        project_id: None,
        // Keep this scope aligned with `ThreadCheckpointLoopExitEvidencePort`,
        // which reconstructs thread scope from `TurnScope` for completion
        // evidence and currently has no owner-user dimension there.
        owner_user_id: None,
        mission_id: None,
    };

    #[cfg(feature = "root-llm-provider")]
    let model_gateway = {
        #[cfg(test)]
        if let Some(gateway) = model_gateway_override {
            gateway
        } else {
            match llm {
                Some(cfg) => build_llm_gateway(cfg)?,
                None => build_stub_gateway(),
            }
        }
        #[cfg(not(test))]
        {
            match llm {
                Some(cfg) => build_llm_gateway(cfg)?,
                None => build_stub_gateway(),
            }
        }
    };
    #[cfg(not(feature = "root-llm-provider"))]
    let model_gateway = {
        #[cfg(test)]
        if let Some(gateway) = model_gateway_override {
            gateway
        } else {
            build_stub_gateway()
        }
        #[cfg(not(test))]
        {
            build_stub_gateway()
        }
    };

    let loop_exit_evidence = Arc::new(ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
        Arc::clone(&thread_service),
        Arc::clone(&turn_state_store) as Arc<dyn ironclaw_turns::TurnStateStore>,
        Arc::clone(&loop_checkpoint_store) as Arc<dyn ironclaw_turns::LoopCheckpointStore>,
        thread_scope.clone(),
    ));

    let composition = build_default_planned_runtime(DefaultPlannedRuntimeParts {
        turn_state: Arc::clone(&turn_state_store),
        thread_service: Arc::clone(&thread_service),
        thread_scope: thread_scope.clone(),
        model_gateway,
        checkpoint_state_store: Arc::clone(&checkpoint_state_store)
            as Arc<dyn ironclaw_turns::CheckpointStateStore>,
        loop_checkpoint_store: Arc::clone(&loop_checkpoint_store)
            as Arc<dyn ironclaw_turns::LoopCheckpointStore>,
        milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
        capability_factory: Arc::new(EmptyCapabilityFactory),
        capability_surface_resolver: Arc::new(AllowAllCapabilitySurfaceResolver),
        loop_exit_evidence,
        config: DefaultPlannedRuntimeConfig {
            worker: TurnRunnerWorkerConfig {
                heartbeat_interval: runner.heartbeat_interval,
                poll_interval: runner.poll_interval,
                scope_filter: None,
            },
            ..DefaultPlannedRuntimeConfig::default()
        },
        model_route_resolver: None,
        cancellation_factory: None,
        skill_context_source: None,
        input_queue: None,
        identity_context_source: Arc::new(EmptyIdentityContextSource),
        model_policy_guard: None,
        model_budget_accountant: None,
        safety_context: None,
    })?;
    let default_run_profile_id = composition
        .run_profile_resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("could not resolve default run profile: {error}"),
        })?
        .profile_id
        .as_str()
        .to_string();

    let worker_cancel = CancellationToken::new();
    let worker = Arc::clone(&composition.worker);
    let worker_cancel_clone = worker_cancel.clone();
    let worker_handle = tokio::spawn(async move {
        worker.run(worker_cancel_clone).await;
    });
    let turn_coordinator: Arc<dyn TurnCoordinator> = composition.coordinator;
    let wake_sender = composition.wake_sender;

    Ok(RebornRuntime {
        services,
        turn_coordinator,
        thread_service,
        thread_scope,
        worker_handle,
        worker_cancel,
        poll_settings: poll,
        actor_user_id,
        source_binding_ref: validated_identity.source_binding_ref,
        reply_target_binding_ref: validated_identity.reply_target_binding_ref,
        default_run_profile_id,
        wake_sender,
        send_locks: Mutex::new(HashMap::new()),
    })
}

struct ValidatedRuntimeIdentity {
    tenant_id: TenantId,
    agent_id: AgentId,
    source_binding_ref: SourceBindingRef,
    reply_target_binding_ref: ReplyTargetBindingRef,
}

fn validate_runtime_identity(
    identity: RebornRuntimeIdentity,
) -> Result<ValidatedRuntimeIdentity, RebornRuntimeError> {
    let tenant_id = TenantId::new(identity.tenant_id).map_err(|reason| {
        RebornRuntimeError::InvalidArgument {
            reason: format!("tenant id: {reason}"),
        }
    })?;
    let agent_id =
        AgentId::new(identity.agent_id).map_err(|reason| RebornRuntimeError::InvalidArgument {
            reason: format!("agent id: {reason}"),
        })?;
    let source_binding_ref =
        SourceBindingRef::new(identity.source_binding_id).map_err(|reason| {
            RebornRuntimeError::InvalidArgument {
                reason: format!("source binding id: {reason}"),
            }
        })?;
    let reply_target_binding_ref = ReplyTargetBindingRef::new(identity.reply_target_binding_id)
        .map_err(|reason| RebornRuntimeError::InvalidArgument {
            reason: format!("reply target binding id: {reason}"),
        })?;
    Ok(ValidatedRuntimeIdentity {
        tenant_id,
        agent_id,
        source_binding_ref,
        reply_target_binding_ref,
    })
}

struct EmptyCapabilityFactory;

#[async_trait::async_trait]
impl LoopCapabilityPortFactory for EmptyCapabilityFactory {
    async fn create_capability_port(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        Ok(Arc::new(EmptyLoopCapabilityPort))
    }
}

struct AllowAllCapabilitySurfaceResolver;

#[async_trait::async_trait]
impl CapabilitySurfaceProfileResolver for AllowAllCapabilitySurfaceResolver {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        Ok(CapabilityAllowSet::All)
    }
}

struct EmptyIdentityContextSource;

#[async_trait::async_trait]
impl HostIdentityContextSource for EmptyIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        Ok(Vec::new())
    }
}

#[cfg(feature = "root-llm-provider")]
fn build_llm_gateway(
    cfg: RebornLlmConfig,
) -> Result<Arc<dyn ironclaw_loop_support::HostManagedModelGateway>, RebornRuntimeError> {
    use ironclaw_llm::{RegistryProviderConfig, config::CacheRetention};
    use ironclaw_reborn::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
    use ironclaw_turns::run_profile::ModelProfileId;

    let protocol = parse_provider_protocol(&cfg.protocol)?;
    let registry_config = RegistryProviderConfig {
        protocol,
        provider_id: cfg.provider_id.clone(),
        api_key: cfg.api_key.clone(),
        base_url: cfg.base_url.clone(),
        model: cfg.model.clone(),
        extra_headers: cfg.extra_headers.clone(),
        oauth_token: None,
        is_codex_chatgpt: false,
        refresh_token: None,
        auth_path: None,
        cache_retention: CacheRetention::None,
        unsupported_params: Vec::new(),
    };
    let provider =
        ironclaw_llm::create_registry_provider(&registry_config, cfg.request_timeout_secs)
            .map_err(|error| RebornRuntimeError::LlmProvider(error.to_string()))?;

    let model_profile_id = ModelProfileId::new("interactive_model").map_err(|reason| {
        RebornRuntimeError::LlmProvider(format!("invalid interactive model profile id: {reason}"))
    })?;
    let policy =
        LlmModelProfilePolicy::new().allow_model_profile(model_profile_id, Some(cfg.model.clone()));
    let gateway = LlmProviderModelGateway::new(provider, policy);
    Ok(Arc::new(gateway))
}

#[cfg(feature = "root-llm-provider")]
fn parse_provider_protocol(
    protocol: &str,
) -> Result<ironclaw_llm::ProviderProtocol, RebornRuntimeError> {
    use ironclaw_llm::ProviderProtocol;

    match protocol {
        "open_ai_completions" | "openai_completions" | "openai" => {
            Ok(ProviderProtocol::OpenAiCompletions)
        }
        "anthropic" => Ok(ProviderProtocol::Anthropic),
        "ollama" => Ok(ProviderProtocol::Ollama),
        "github_copilot" => Ok(ProviderProtocol::GithubCopilot),
        "deep_seek" | "deepseek" => Ok(ProviderProtocol::DeepSeek),
        "gemini" => Ok(ProviderProtocol::Gemini),
        "open_router" | "openrouter" => Ok(ProviderProtocol::OpenRouter),
        _ => Err(RebornRuntimeError::LlmProvider(format!(
            "unsupported llm protocol: {protocol}"
        ))),
    }
}

fn build_stub_gateway() -> Arc<dyn ironclaw_loop_support::HostManagedModelGateway> {
    use async_trait::async_trait;
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
        HostManagedModelRequest, HostManagedModelResponse,
    };

    #[derive(Debug, Default)]
    struct StubGateway;

    #[async_trait]
    impl HostManagedModelGateway for StubGateway {
        async fn stream_model(
            &self,
            _request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::Unavailable,
                "no LLM gateway wired (build with `root-llm-provider` feature)",
            ))
        }
    }

    Arc::new(StubGateway)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
        HostManagedModelResponse,
    };
    use ironclaw_turns::TurnStatus;

    use crate::input::RebornBuildInput;
    use crate::runtime_input::{PollSettings, RebornRuntimeIdentity, RebornRuntimeInput};

    use super::build_reborn_runtime;

    #[derive(Debug)]
    struct RecordingGateway {
        reply: String,
        requests: Arc<StdMutex<Vec<HostManagedModelRequest>>>,
    }

    #[async_trait]
    impl HostManagedModelGateway for RecordingGateway {
        async fn stream_model(
            &self,
            request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            self.requests
                .lock()
                .expect("recording gateway requests lock poisoned")
                .push(request);
            Ok(HostManagedModelResponse::assistant_reply(
                self.reply.clone(),
            ))
        }
    }

    #[tokio::test]
    async fn send_user_message_returns_completed_assistant_text_with_recording_gateway() {
        let root = tempfile::tempdir().expect("tempdir");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "recorded runtime reply".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(RebornBuildInput::local_dev(
            "runtime-success-owner",
            root.path().join("local-dev"),
        ))
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-success-tenant".to_string(),
            agent_id: "runtime-success-agent".to_string(),
            source_binding_id: "runtime-success-source".to_string(),
            reply_target_binding_id: "runtime-success-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            Duration::from_secs(3),
            runtime.send_user_message(&conversation, "ping"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("recorded runtime reply"));
        assert_eq!(
            requests
                .lock()
                .expect("recording gateway requests lock poisoned")
                .len(),
            1
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }
}

#[cfg(all(test, feature = "root-llm-provider"))]
mod llm_provider_tests {
    use ironclaw_llm::ProviderProtocol;

    use super::parse_provider_protocol;

    #[test]
    fn parses_supported_provider_protocols_without_wildcard_mapping() {
        assert_eq!(
            parse_provider_protocol("open_ai_completions").unwrap(),
            ProviderProtocol::OpenAiCompletions
        );
        assert_eq!(
            parse_provider_protocol("openai_completions").unwrap(),
            ProviderProtocol::OpenAiCompletions
        );
        assert_eq!(
            parse_provider_protocol("openai").unwrap(),
            ProviderProtocol::OpenAiCompletions
        );
        assert_eq!(
            parse_provider_protocol("anthropic").unwrap(),
            ProviderProtocol::Anthropic
        );
        assert_eq!(
            parse_provider_protocol("ollama").unwrap(),
            ProviderProtocol::Ollama
        );
        assert_eq!(
            parse_provider_protocol("deep_seek").unwrap(),
            ProviderProtocol::DeepSeek
        );
        assert_eq!(
            parse_provider_protocol("deepseek").unwrap(),
            ProviderProtocol::DeepSeek
        );
        assert_eq!(
            parse_provider_protocol("gemini").unwrap(),
            ProviderProtocol::Gemini
        );
        assert_eq!(
            parse_provider_protocol("open_router").unwrap(),
            ProviderProtocol::OpenRouter
        );
        assert_eq!(
            parse_provider_protocol("openrouter").unwrap(),
            ProviderProtocol::OpenRouter
        );
        assert_eq!(
            parse_provider_protocol("github_copilot").unwrap(),
            ProviderProtocol::GithubCopilot
        );
    }

    #[test]
    fn rejects_unsupported_provider_protocol() {
        assert!(parse_provider_protocol("made_up_protocol").is_err());
    }
}
