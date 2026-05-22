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

use ironclaw_filesystem::LocalFilesystem;
use ironclaw_first_party_extensions::{
    FirstPartySkillsExtension, FirstPartySkillsExtensionHandles, LoadedFirstPartyExtensions,
    SelectableSkillContextSource, SkillActivationMode as FirstPartySkillActivationMode,
    SkillActivationPlan, SkillActivationRequest as FirstPartySkillActivationRequest,
    SkillActivationSelectorConfig, SkillBundleAsset as FirstPartySkillBundleAsset,
    SkillBundleAssetReadError, SkillExecutionAdapter,
};
use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    FilesystemSkillBundleSource, HostIdentityContextBuildError, HostIdentityContextCandidate,
    HostIdentityContextSource, HostSkillContextSource, SkillBundleId, SkillSourceKind,
};
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
    ReplyTargetBindingRef, RunProfileResolutionRequest, SanitizedCancelReason, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnId,
    TurnRunId, TurnScope, TurnStatus,
    run_profile::{InMemoryLoopHostMilestoneSink, LoopRunContext, PromptMode, ResolvedRunProfile},
};

use crate::runtime_input::{PollSettings, RebornRuntimeIdentity, RebornRuntimeInput};
use crate::{RebornBuildError, RebornCompositionProfile, RebornServices, build_reborn_services};

mod local_dev;

#[cfg(feature = "root-llm-provider")]
use crate::runtime_input::{ResolvedRebornLlm, ResolvedRebornLlmSource};

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
    #[error("skill execution unavailable for assembled runtime")]
    SkillExecutionUnavailable,
    #[error("skill execution failed: {0}")]
    SkillExecution(String),
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
    default_resolved_run_profile: ResolvedRunProfile,
    wake_sender: TurnRunnerWakeSender,
    send_locks: Mutex<HashMap<ConversationId, Arc<Mutex<()>>>>,
    skill_activation_source: Option<Arc<LocalDevSelectableSkillContextSource>>,
    skill_execution_adapter: Option<Arc<LocalDevSkillExecutionAdapter>>,
}

type LocalDevSelectableSkillContextSource =
    SelectableSkillContextSource<FilesystemSkillBundleSource<LocalFilesystem>>;
type LocalDevSkillExecutionAdapter =
    SkillExecutionAdapter<FilesystemSkillBundleSource<LocalFilesystem>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSkillExecutionPlan {
    activations: Vec<RebornSkillActivation>,
    rewritten_message: String,
    feedback: Vec<String>,
    active_bundles: Vec<RebornSkillBundle>,
    asset_scope_bundles: Vec<SkillBundleId>,
}

impl RebornSkillExecutionPlan {
    pub fn activations(&self) -> &[RebornSkillActivation] {
        &self.activations
    }

    pub fn rewritten_message(&self) -> &str {
        &self.rewritten_message
    }

    pub fn feedback(&self) -> &[String] {
        &self.feedback
    }

    pub fn active_bundles(&self) -> &[RebornSkillBundle] {
        &self.active_bundles
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSkillExecutionResult {
    pub plan: RebornSkillExecutionPlan,
    pub reply: AssistantReply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSkillActivation {
    pub name: String,
    pub source: Option<RebornSkillSourceKind>,
    pub mode: RebornSkillActivationMode,
    bundle_id: Option<SkillBundleId>,
}

impl RebornSkillActivation {
    fn into_first_party_request(self) -> FirstPartySkillActivationRequest {
        FirstPartySkillActivationRequest {
            name: self.name,
            source: self.source.map(SkillSourceKind::from),
            bundle_id: self.bundle_id,
            mode: FirstPartySkillActivationMode::from(self.mode),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSkillBundle {
    pub source: RebornSkillSourceKind,
    pub skill_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RebornSkillSourceKind {
    System,
    TenantShared,
    User,
}

impl From<SkillSourceKind> for RebornSkillSourceKind {
    fn from(value: SkillSourceKind) -> Self {
        match value {
            SkillSourceKind::System => Self::System,
            SkillSourceKind::TenantShared => Self::TenantShared,
            SkillSourceKind::User => Self::User,
        }
    }
}

impl From<RebornSkillSourceKind> for SkillSourceKind {
    fn from(value: RebornSkillSourceKind) -> Self {
        match value {
            RebornSkillSourceKind::System => Self::System,
            RebornSkillSourceKind::TenantShared => Self::TenantShared,
            RebornSkillSourceKind::User => Self::User,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RebornSkillActivationMode {
    ExplicitMention,
    ActivationCriteria,
}

impl From<FirstPartySkillActivationMode> for RebornSkillActivationMode {
    fn from(value: FirstPartySkillActivationMode) -> Self {
        match value {
            FirstPartySkillActivationMode::ExplicitMention => Self::ExplicitMention,
            FirstPartySkillActivationMode::ActivationCriteria => Self::ActivationCriteria,
        }
    }
}

impl From<RebornSkillActivationMode> for FirstPartySkillActivationMode {
    fn from(value: RebornSkillActivationMode) -> Self {
        match value {
            RebornSkillActivationMode::ExplicitMention => Self::ExplicitMention,
            RebornSkillActivationMode::ActivationCriteria => Self::ActivationCriteria,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSkillAsset {
    pub source: RebornSkillSourceKind,
    pub skill_name: String,
    pub path: String,
    pub bytes: Vec<u8>,
}

impl RebornSkillAsset {
    pub fn into_utf8(self) -> Result<String, std::string::FromUtf8Error> {
        String::from_utf8(self.bytes)
    }
}

impl From<SkillActivationPlan> for RebornSkillExecutionPlan {
    fn from(value: SkillActivationPlan) -> Self {
        let asset_scope_bundles = value.activated_bundles().to_vec();
        let active_bundles = value
            .activated_bundles()
            .iter()
            .map(RebornSkillBundle::from)
            .collect();
        Self {
            activations: value
                .selection
                .activations
                .into_iter()
                .map(RebornSkillActivation::from)
                .collect(),
            rewritten_message: value.selection.rewritten_message,
            feedback: value.selection.feedback,
            active_bundles,
            asset_scope_bundles,
        }
    }
}

impl From<FirstPartySkillActivationRequest> for RebornSkillActivation {
    fn from(value: FirstPartySkillActivationRequest) -> Self {
        Self {
            name: value.name,
            source: value.source.map(RebornSkillSourceKind::from),
            mode: RebornSkillActivationMode::from(value.mode),
            bundle_id: value.bundle_id,
        }
    }
}

impl From<&SkillBundleId> for RebornSkillBundle {
    fn from(value: &SkillBundleId) -> Self {
        Self {
            source: RebornSkillSourceKind::from(value.source_kind()),
            skill_name: value.name().to_string(),
        }
    }
}

impl From<FirstPartySkillBundleAsset> for RebornSkillAsset {
    fn from(value: FirstPartySkillBundleAsset) -> Self {
        Self {
            source: RebornSkillSourceKind::from(value.bundle_id.source_kind()),
            skill_name: value.bundle_id.name().to_string(),
            path: value.path.as_str().to_string(),
            bytes: value.bytes,
        }
    }
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

    pub(crate) fn webui_thread_service(&self) -> Arc<dyn SessionThreadService> {
        self.thread_service.clone()
    }

    pub(crate) fn webui_turn_coordinator(&self) -> Arc<dyn TurnCoordinator> {
        self.turn_coordinator.clone()
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
        if let Some(skill_activation_source) = &self.skill_activation_source {
            skill_activation_source.record_user_message(scope.clone(), text);
        }
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

    /// Submit a skill-aware message through the normal Reborn loop and return
    /// the structured activation plan produced during prompt construction.
    pub async fn execute_skill_message(
        &self,
        conversation: &ConversationId,
        text: &str,
    ) -> Result<RebornSkillExecutionResult, RebornRuntimeError> {
        let adapter = self
            .skill_execution_adapter
            .as_ref()
            .ok_or(RebornRuntimeError::SkillExecutionUnavailable)?;
        let scope = self.turn_scope_for(&conversation.0);
        adapter.capture_next_activation_plan(scope.clone());
        let reply = match self.send_user_message(conversation, text).await {
            Ok(reply) => reply,
            Err(error) => {
                adapter.cancel_next_activation_plan_capture(&scope);
                return Err(error);
            }
        };
        let plan = self.skill_execution_plan_for_run(&scope, reply.run_id)?;
        Ok(RebornSkillExecutionResult { plan, reply })
    }

    /// Read a bundle-relative asset from a skill activated by
    /// [`Self::execute_skill_message`].
    pub async fn read_skill_execution_asset(
        &self,
        conversation: &ConversationId,
        plan: &RebornSkillExecutionPlan,
        activation: &RebornSkillActivation,
        path: impl AsRef<str>,
    ) -> Result<RebornSkillAsset, RebornRuntimeError> {
        let adapter = self
            .skill_execution_adapter
            .as_ref()
            .ok_or(RebornRuntimeError::SkillExecutionUnavailable)?;
        let run_context = self.skill_execution_run_context(conversation);
        let reader = ironclaw_first_party_extensions::SkillBundleAssetReader::new(
            adapter.selector().bundle_source(),
            plan.asset_scope_bundles.iter().cloned(),
        );
        reader
            .read_file_for_activation(
                &run_context,
                &activation.clone().into_first_party_request(),
                path,
            )
            .await
            .map(RebornSkillAsset::from)
            .map_err(skill_asset_error)
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

    fn skill_execution_plan_for_run(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<RebornSkillExecutionPlan, RebornRuntimeError> {
        let adapter = self
            .skill_execution_adapter
            .as_ref()
            .ok_or(RebornRuntimeError::SkillExecutionUnavailable)?;
        adapter
            .take_activation_plan_for_run(scope, run_id)
            .map(RebornSkillExecutionPlan::from)
            .ok_or_else(|| {
                RebornRuntimeError::SkillExecution("skill activation plan unavailable".to_string())
            })
    }

    fn skill_execution_run_context(&self, conversation: &ConversationId) -> LoopRunContext {
        LoopRunContext::new(
            self.turn_scope_for(&conversation.0),
            TurnId::new(),
            TurnRunId::new(),
            self.default_resolved_run_profile.clone(),
        )
        .with_actor(TurnActor::new(self.actor_user_id.clone()))
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
        skill_context_source: configured_skill_context_source,
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
    if services_input.runtime_policy().is_none() {
        return Err(RebornRuntimeError::InvalidArgument {
            reason: "RebornRuntimeInput.services must include a resolved runtime policy"
                .to_string(),
        });
    }

    let owner_id = services_input.owner_id().to_string();
    let mut services = build_reborn_services(services_input).await?;

    let local_runtime =
        services
            .local_runtime
            .as_ref()
            .ok_or(RebornRuntimeError::InvalidArgument {
                reason: "local-dev RebornServices did not provide runtime substrate".to_string(),
            })?;
    let turn_state_store = Arc::clone(&local_runtime.turn_state);
    let checkpoint_state_store = Arc::clone(&local_runtime.checkpoint_state_store);
    let loop_checkpoint_store = Arc::clone(&local_runtime.loop_checkpoint_store);
    let thread_service = Arc::clone(&local_runtime.thread_service);
    let (skill_context_source, skill_activation_source, skill_execution_adapter) =
        match configured_skill_context_source {
            Some(source) => (Some(source), None, None),
            None => {
                let local_dev_skills = local_dev_filesystem_skill_context_source(local_runtime)?;
                (
                    Some(local_dev_skills.source),
                    Some(local_dev_skills.activation_source),
                    Some(local_dev_skills.execution_adapter),
                )
            }
        };

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
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let local_dev_capabilities = local_dev::capability_wiring(
        &services,
        actor_user_id.clone(),
        model_gateway,
        Some(milestone_sink.clone()),
    )
    .ok_or(RebornRuntimeError::HostRuntimeUnavailable)?;
    let capability_factory = local_dev_capabilities.capability_factory;
    let model_gateway = local_dev_capabilities.model_gateway;

    let composition = build_default_planned_runtime(DefaultPlannedRuntimeParts {
        turn_state: Arc::clone(&turn_state_store),
        thread_service: Arc::clone(&thread_service),
        thread_scope: thread_scope.clone(),
        model_gateway,
        checkpoint_state_store: Arc::clone(&checkpoint_state_store)
            as Arc<dyn ironclaw_turns::CheckpointStateStore>,
        loop_checkpoint_store: Arc::clone(&loop_checkpoint_store)
            as Arc<dyn ironclaw_turns::LoopCheckpointStore>,
        milestone_sink,
        capability_factory,
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
        skill_context_source,
        input_queue: None,
        identity_context_source: Arc::new(EmptyIdentityContextSource),
        model_policy_guard: None,
        model_budget_accountant: None,
        safety_context: None,
    })?;
    let default_resolved_run_profile = composition
        .run_profile_resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("could not resolve default run profile: {error}"),
        })?;
    let default_run_profile_id = default_resolved_run_profile.profile_id.as_str().to_string();
    let planned_turn_coordinator: Arc<dyn TurnCoordinator> = composition.coordinator.clone();
    services.turn_coordinator = Some(Arc::clone(&planned_turn_coordinator));

    let worker_cancel = CancellationToken::new();
    let worker = Arc::clone(&composition.worker);
    let worker_cancel_clone = worker_cancel.clone();
    let worker_handle = tokio::spawn(async move {
        worker.run(worker_cancel_clone).await;
    });
    let turn_coordinator = planned_turn_coordinator;
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
        default_resolved_run_profile,
        wake_sender,
        send_locks: Mutex::new(HashMap::new()),
        skill_activation_source,
        skill_execution_adapter,
    })
}

struct LocalDevSkillContextSource {
    source: Arc<dyn HostSkillContextSource>,
    activation_source: Arc<LocalDevSelectableSkillContextSource>,
    execution_adapter: Arc<LocalDevSkillExecutionAdapter>,
}

fn local_dev_filesystem_skill_context_source(
    local_runtime: &crate::factory::RebornLocalRuntimeServices,
) -> Result<LocalDevSkillContextSource, RebornRuntimeError> {
    let skills_extension = FirstPartySkillsExtension::new(
        Arc::clone(&local_runtime.skill_filesystem),
        FirstPartySkillsExtensionHandles::reborn_default().map_err(|reason| {
            RebornRuntimeError::InvalidArgument {
                reason: format!("first-party skills extension handles: {reason}"),
            }
        })?,
    );
    let loaded_extensions = LoadedFirstPartyExtensions::new().with_skills(skills_extension);
    let activation_source = loaded_extensions
        .selectable_skill_context_source(SkillActivationSelectorConfig::default())
        .ok_or_else(|| RebornRuntimeError::InvalidArgument {
            reason: "first-party skills extension did not expose a skill context source"
                .to_string(),
        })?;
    let execution_adapter = Arc::new(SkillExecutionAdapter::new(Arc::clone(&activation_source)));
    let source: Arc<dyn HostSkillContextSource> = activation_source.clone();
    Ok(LocalDevSkillContextSource {
        source,
        activation_source,
        execution_adapter,
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

fn skill_asset_error(error: SkillBundleAssetReadError) -> RebornRuntimeError {
    RebornRuntimeError::SkillExecution(error.to_string())
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
    llm: ResolvedRebornLlm,
) -> Result<Arc<dyn ironclaw_loop_support::HostManagedModelGateway>, RebornRuntimeError> {
    use ironclaw_llm::RegistryProviderConfig;
    use ironclaw_reborn::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
    use ironclaw_turns::run_profile::ModelProfileId;

    let model = llm.model().to_string();
    let provider = match llm.source {
        ResolvedRebornLlmSource::Catalog(cfg) => {
            let protocol = parse_provider_protocol(&cfg.protocol)?;
            let registry_config = RegistryProviderConfig::generic(
                protocol,
                cfg.provider_id.clone(),
                cfg.api_key.clone(),
                cfg.base_url.clone(),
                cfg.model.clone(),
            )
            .with_extra_headers(cfg.extra_headers.clone());
            ironclaw_llm::create_registry_provider(&registry_config, cfg.request_timeout_secs)
        }
        ResolvedRebornLlmSource::RegistryProvider {
            config,
            request_timeout_secs,
        } => ironclaw_llm::create_registry_provider(&config, request_timeout_secs),
    }
    .map_err(|error| RebornRuntimeError::LlmProvider(error.to_string()))?;

    let model_profile_id = ModelProfileId::new("interactive_model").map_err(|reason| {
        RebornRuntimeError::LlmProvider(format!("invalid interactive model profile id: {reason}"))
    })?;
    let policy = LlmModelProfilePolicy::new().allow_model_profile(model_profile_id, Some(model));
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
    use std::sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use async_trait::async_trait;
    use ironclaw_host_api::{
        CapabilityId,
        runtime_policy::{
            ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy,
            FilesystemBackendKind, NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
        },
    };
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
        HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
        HostSkillContextBuildError, HostSkillContextCandidate, HostSkillContextSource,
    };
    use ironclaw_skills::SkillTrust;
    use ironclaw_threads::{
        LoadContextMessagesRequest, MessageKind, SessionThreadService, ThreadHistoryRequest,
    };
    use ironclaw_turns::{
        TurnStatus,
        run_profile::{
            LoopCapabilityPort, LoopRunContext, ProviderToolCall, SkillVisibility,
            VisibleCapabilityRequest,
        },
    };

    use crate::RebornReadinessState;
    use crate::input::RebornBuildInput;
    use crate::runtime_input::{PollSettings, RebornRuntimeIdentity, RebornRuntimeInput};
    use crate::webui::build_webui_services;
    use crate::{
        RebornSkillActivation, RebornSkillActivationMode, RebornSkillExecutionPlan,
        RebornSkillSourceKind,
    };

    use super::{RebornRuntimeError, build_reborn_runtime};

    fn local_dev_runtime_policy() -> EffectiveRuntimePolicy {
        EffectiveRuntimePolicy {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalDev,
            resolved_profile: RuntimeProfile::LocalDev,
            filesystem_backend: FilesystemBackendKind::HostWorkspace,
            process_backend: ProcessBackendKind::LocalHost,
            network_mode: NetworkMode::DirectLogged,
            secret_mode: SecretMode::ScrubbedEnv,
            approval_policy: ApprovalPolicy::AskDestructive,
            audit_mode: AuditMode::LocalMinimal,
        }
    }

    #[derive(Debug)]
    struct RecordingGateway {
        reply: String,
        requests: Arc<StdMutex<Vec<HostManagedModelRequest>>>,
    }

    #[derive(Debug, Default)]
    struct FailingSkillContextSource {
        calls: AtomicUsize,
    }

    #[derive(Debug, Default)]
    struct ToolCallingGateway {
        calls: StdMutex<usize>,
        stream_model_calls: StdMutex<usize>,
        requests: StdMutex<Vec<HostManagedModelRequest>>,
    }

    #[derive(Debug, Default)]
    struct WorkspaceListingGateway {
        calls: StdMutex<usize>,
        requests: StdMutex<Vec<HostManagedModelRequest>>,
    }

    struct StaticSkillContextSource {
        candidates: Vec<HostSkillContextCandidate>,
    }

    impl StaticSkillContextSource {
        fn new(candidates: Vec<HostSkillContextCandidate>) -> Self {
            Self { candidates }
        }
    }

    #[async_trait]
    impl HostSkillContextSource for StaticSkillContextSource {
        async fn load_skill_context_candidates(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Vec<HostSkillContextCandidate>, HostSkillContextBuildError> {
            Ok(self.candidates.clone())
        }
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

    #[async_trait]
    impl HostSkillContextSource for FailingSkillContextSource {
        async fn load_skill_context_candidates(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Vec<HostSkillContextCandidate>, HostSkillContextBuildError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(HostSkillContextBuildError::SourceUnavailable)
        }
    }

    #[async_trait]
    impl HostManagedModelGateway for ToolCallingGateway {
        async fn stream_model(
            &self,
            request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            *self
                .stream_model_calls
                .lock()
                .expect("tool gateway stream count lock poisoned") += 1;
            self.requests
                .lock()
                .expect("tool gateway requests lock poisoned")
                .push(request);
            Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidRequest,
                "expected capability-aware model path",
            ))
        }

        async fn stream_model_with_capabilities(
            &self,
            request: HostManagedModelRequest,
            capabilities: Arc<dyn LoopCapabilityPort>,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            let call_index = {
                let mut calls = self.calls.lock().expect("tool gateway lock poisoned");
                let call_index = *calls;
                *calls += 1;
                call_index
            };
            self.requests
                .lock()
                .expect("tool gateway requests lock poisoned")
                .push(request.clone());
            if call_index > 0 {
                let tool_result = request
                    .messages
                    .iter()
                    .find(|message| message.role == HostManagedModelMessageRole::ToolResult)
                    .expect("second model call should include tool result");
                assert!(
                    tool_result.content.contains("hello from tool"),
                    "tool result should expose hydrated capability output, got {}",
                    tool_result.content
                );
                let provider_call = tool_result
                    .tool_result_provider_call
                    .as_ref()
                    .expect("provider replay metadata");
                assert_eq!(provider_call.provider_call_id, "call-1");
                assert_eq!(
                    provider_call.capability_id,
                    CapabilityId::new("builtin.echo").unwrap()
                );
                return Ok(HostManagedModelResponse::assistant_reply("tool ok"));
            }

            let surface = capabilities
                .visible_capabilities(VisibleCapabilityRequest)
                .await
                .map_err(model_capability_error)?;
            let echo_id = CapabilityId::new("builtin.echo").expect("echo id");
            assert!(
                surface
                    .descriptors
                    .iter()
                    .any(|descriptor| descriptor.capability_id == echo_id),
                "builtin echo must be visible through local-dev runtime capability surface"
            );
            let echo_tool = capabilities
                .tool_definitions()
                .map_err(model_capability_error)?
                .into_iter()
                .find(|definition| definition.capability_id == echo_id)
                .expect("echo provider tool definition");
            let candidate = capabilities
                .register_provider_tool_call(ProviderToolCall {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    turn_id: Some("provider-turn-1".to_string()),
                    id: "call-1".to_string(),
                    name: echo_tool.name,
                    arguments: serde_json::json!({"message": "hello from tool"}),
                    response_reasoning: None,
                    reasoning: None,
                    signature: None,
                })
                .await
                .map_err(model_capability_error)?;
            Ok(HostManagedModelResponse::capability_calls(
                vec![candidate],
                "",
            ))
        }
    }

    #[async_trait]
    impl HostManagedModelGateway for WorkspaceListingGateway {
        async fn stream_model(
            &self,
            request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            self.requests
                .lock()
                .expect("workspace gateway requests lock poisoned")
                .push(request);
            Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidRequest,
                "expected capability-aware model path",
            ))
        }

        async fn stream_model_with_capabilities(
            &self,
            request: HostManagedModelRequest,
            capabilities: Arc<dyn LoopCapabilityPort>,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            let call_index = {
                let mut calls = self.calls.lock().expect("workspace gateway lock poisoned");
                let call_index = *calls;
                *calls += 1;
                call_index
            };
            self.requests
                .lock()
                .expect("workspace gateway requests lock poisoned")
                .push(request.clone());
            if call_index > 0 {
                let tool_result = request
                    .messages
                    .iter()
                    .find(|message| message.role == HostManagedModelMessageRole::ToolResult)
                    .expect("second model call should include tool result");
                assert!(
                    tool_result.content.contains("workspace-sentinel.txt"),
                    "workspace listing should expose configured workspace root, got {}",
                    tool_result.content
                );
                return Ok(HostManagedModelResponse::assistant_reply("workspace ok"));
            }

            let list_dir_id = CapabilityId::new("builtin.list_dir").expect("list_dir id");
            let list_dir_tool = capabilities
                .tool_definitions()
                .map_err(model_capability_error)?
                .into_iter()
                .find(|definition| definition.capability_id == list_dir_id)
                .expect("list_dir provider tool definition");
            let candidate = capabilities
                .register_provider_tool_call(ProviderToolCall {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    turn_id: Some("provider-turn-1".to_string()),
                    id: "call-1".to_string(),
                    name: list_dir_tool.name,
                    arguments: serde_json::json!({"path": "/workspace"}),
                    response_reasoning: None,
                    reasoning: None,
                    signature: None,
                })
                .await
                .map_err(model_capability_error)?;
            Ok(HostManagedModelResponse::capability_calls(
                vec![candidate],
                "",
            ))
        }
    }

    fn model_capability_error(error: impl std::fmt::Display) -> HostManagedModelError {
        HostManagedModelError::safe(HostManagedModelErrorKind::Unavailable, error.to_string())
    }

    fn skill_md(name: &str, description: &str, prompt: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: {description}\nactivation:\n  keywords: [\"{name}\"]\n---\n\n{prompt}"
        )
    }

    #[tokio::test]
    async fn send_user_message_returns_completed_assistant_text_with_recording_gateway() {
        let root = tempfile::tempdir().expect("tempdir");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "recorded runtime reply".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-success-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
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
        let local_runtime = runtime
            .services
            .local_runtime
            .as_ref()
            .expect("runtime should use local-dev RebornServices substrate");
        assert!(
            Arc::ptr_eq(&runtime.thread_service, &local_runtime.thread_service),
            "REPL runtime should use the thread service owned by RebornServices"
        );
        assert!(
            Arc::ptr_eq(
                &runtime.turn_coordinator,
                runtime
                    .services
                    .turn_coordinator
                    .as_ref()
                    .expect("RebornServices turn coordinator")
            ),
            "REPL runtime should drive turns through RebornServices"
        );
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

    #[tokio::test]
    async fn send_user_message_uses_caller_supplied_skill_context_source() {
        let root = tempfile::tempdir().expect("tempdir");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "should not reach model".to_string(),
            requests: Arc::clone(&requests),
        });
        let skill_context_source = Arc::new(FailingSkillContextSource::default());
        let skill_context_source_for_input: Arc<dyn HostSkillContextSource> =
            skill_context_source.clone();
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-skill-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-skill-tenant".to_string(),
            agent_id: "runtime-skill-agent".to_string(),
            source_binding_id: "runtime-skill-source".to_string(),
            reply_target_binding_id: "runtime-skill-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_skill_context_source(skill_context_source_for_input)
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

        assert_ne!(reply.status, TurnStatus::Completed);
        assert_eq!(
            skill_context_source.calls.load(Ordering::SeqCst),
            1,
            "composition should pass caller-supplied skill context into the planned runtime"
        );
        assert!(
            requests
                .lock()
                .expect("recording gateway requests lock poisoned")
                .is_empty(),
            "skill context failure should stop before model dispatch"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_exposes_host_runtime_capabilities_to_model_calls() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(ToolCallingGateway::default());
        let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-tools-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-tools-tenant".to_string(),
            agent_id: "runtime-tools-agent".to_string(),
            source_binding_id: "runtime-tools-source".to_string(),
            reply_target_binding_id: "runtime-tools-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway_for_runtime);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            Duration::from_secs(3),
            runtime.send_user_message(&conversation, "use echo tool"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("tool ok"));
        assert_eq!(
            *gateway
                .stream_model_calls
                .lock()
                .expect("tool gateway stream count lock poisoned"),
            0,
            "runtime should use capability-aware model path"
        );
        assert_eq!(
            gateway
                .requests
                .lock()
                .expect("tool gateway requests lock poisoned")
                .len(),
            2,
            "tool call should require initial request plus tool-result follow-up"
        );
        let history = runtime
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: runtime.thread_scope.clone(),
                thread_id: conversation.0.clone(),
            })
            .await
            .expect("thread history");
        let tool_result = history
            .messages
            .iter()
            .find(|message| message.kind == MessageKind::ToolResultReference)
            .expect("tool result reference should persist in thread history");
        assert!(
            tool_result
                .tool_result_ref
                .as_deref()
                .is_some_and(|result_ref| result_ref.starts_with("result:")),
            "tool result should persist a durable result ref"
        );
        assert!(
            tool_result.tool_result_provider_call.is_none(),
            "product thread history should scrub provider replay metadata"
        );
        let context = runtime
            .thread_service
            .load_context_messages(LoadContextMessagesRequest {
                scope: runtime.thread_scope.clone(),
                thread_id: conversation.0.clone(),
                message_ids: vec![tool_result.message_id],
            })
            .await
            .expect("tool result context");
        let provider_call = context.messages[0]
            .tool_result_provider_call
            .as_ref()
            .expect("model context should preserve provider replay metadata");
        assert_eq!(provider_call.provider_call_id, "call-1");
        assert_eq!(
            provider_call.capability_id,
            CapabilityId::new("builtin.echo").unwrap()
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_wires_input_skill_context_source_to_model_calls() {
        let root = tempfile::tempdir().expect("tempdir");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "skill context ok".to_string(),
            requests: Arc::clone(&requests),
        });
        let skill_source = Arc::new(StaticSkillContextSource::new(vec![
            HostSkillContextCandidate::new(
                skill_md(
                    "review-helper",
                    "review helper description",
                    "Use review helper prompt content.",
                ),
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Visible),
            ),
        ]));
        let skill_context_source: Arc<dyn HostSkillContextSource> = skill_source;
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-skill-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-skill-tenant".to_string(),
            agent_id: "runtime-skill-agent".to_string(),
            source_binding_id: "runtime-skill-source".to_string(),
            reply_target_binding_id: "runtime-skill-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_skill_context_source(skill_context_source)
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            Duration::from_secs(3),
            runtime.send_user_message(&conversation, "review this"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("skill context ok"));
        let (request_count, skill_message_content) = {
            let requests = requests
                .lock()
                .expect("recording gateway requests lock poisoned");
            let skill_message = requests[0]
                .messages
                .iter()
                .find(|message| {
                    message.role == HostManagedModelMessageRole::System
                        && message
                            .content_ref
                            .as_str()
                            .starts_with("msg:snippet.skill.review-helper.")
                })
                .expect("model request should include skill-context system message");
            (requests.len(), skill_message.content.clone())
        };
        assert_eq!(request_count, 1);
        assert!(skill_message_content.contains("review helper description"));
        assert!(skill_message_content.contains("Use review helper prompt content."));

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_selects_filesystem_skills_by_message_before_model_calls() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/skills/system-helper"))
            .expect("system skill dir");
        std::fs::write(
            storage_root.join("system/skills/system-helper/SKILL.md"),
            skill_md(
                "system-helper",
                "system helper description",
                "SYSTEM_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write system skill");
        std::fs::create_dir_all(storage_root.join("skills/local-helper")).expect("user skill dir");
        std::fs::write(
            storage_root.join("skills/local-helper/SKILL.md"),
            skill_md(
                "local-helper",
                "local helper description",
                "USER_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write user skill");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "filesystem skill context ok".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-filesystem-skill-owner", storage_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-filesystem-skill-tenant".to_string(),
            agent_id: "runtime-filesystem-skill-agent".to_string(),
            source_binding_id: "runtime-filesystem-skill-source".to_string(),
            reply_target_binding_id: "runtime-filesystem-skill-reply".to_string(),
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
            runtime.send_user_message(&conversation, "please use system-helper"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("filesystem skill context ok"));
        let skill_messages = {
            let requests = requests
                .lock()
                .expect("recording gateway requests lock poisoned");
            requests[0]
                .messages
                .iter()
                .filter(|message| {
                    message.role == HostManagedModelMessageRole::System
                        && message
                            .content_ref
                            .as_str()
                            .starts_with("msg:snippet.skill.")
                })
                .map(|message| message.content.clone())
                .collect::<Vec<_>>()
        };
        let combined_skill_context = skill_messages.join("\n");
        assert_eq!(skill_messages.len(), 1);
        assert!(combined_skill_context.contains("system helper description"));
        assert!(combined_skill_context.contains("SYSTEM_HELPER_PROMPT_SENTINEL"));
        assert!(!combined_skill_context.contains("local helper description"));
        assert!(!combined_skill_context.contains("USER_HELPER_PROMPT_SENTINEL"));

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn execute_skill_message_returns_plan_and_reads_active_bundle_assets() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/skills/system-helper/references"))
            .expect("system skill references dir");
        std::fs::write(
            storage_root.join("system/skills/system-helper/SKILL.md"),
            skill_md(
                "system-helper",
                "system helper description",
                "SYSTEM_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write system skill");
        std::fs::write(
            storage_root.join("system/skills/system-helper/references/policy.md"),
            "SYSTEM_HELPER_POLICY_SENTINEL",
        )
        .expect("write system skill policy");
        std::fs::create_dir_all(storage_root.join("skills/local-helper/references"))
            .expect("user skill references dir");
        std::fs::write(
            storage_root.join("skills/local-helper/SKILL.md"),
            skill_md(
                "local-helper",
                "local helper description",
                "USER_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write user skill");
        std::fs::write(
            storage_root.join("skills/local-helper/references/policy.md"),
            "USER_HELPER_POLICY_SENTINEL",
        )
        .expect("write user skill policy");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "skill execution ok".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-skill-execution-owner", storage_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-skill-execution-tenant".to_string(),
            agent_id: "runtime-skill-execution-agent".to_string(),
            source_binding_id: "runtime-skill-execution-source".to_string(),
            reply_target_binding_id: "runtime-skill-execution-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            runtime.execute_skill_message(&conversation, "please use system-helper"),
        )
        .await
        .expect("runtime skill execution should finish")
        .expect("runtime skill execution should succeed");

        assert_eq!(result.reply.status, TurnStatus::Completed);
        assert_eq!(result.reply.text.as_deref(), Some("skill execution ok"));
        assert_eq!(result.plan.activations().len(), 1);
        let activation = &result.plan.activations()[0];
        assert_eq!(activation.name, "system-helper");
        assert_eq!(activation.source, Some(RebornSkillSourceKind::System));
        assert_eq!(result.plan.active_bundles().len(), 1);
        assert_eq!(
            result.plan.active_bundles()[0].source,
            RebornSkillSourceKind::System
        );
        assert_eq!(result.plan.active_bundles()[0].skill_name, "system-helper");

        let asset = runtime
            .read_skill_execution_asset(
                &conversation,
                &result.plan,
                activation,
                "references/policy.md",
            )
            .await
            .expect("active skill asset");
        assert_eq!(
            asset.into_utf8().expect("asset utf8"),
            "SYSTEM_HELPER_POLICY_SENTINEL"
        );

        let inactive = RebornSkillActivation {
            name: "local-helper".to_string(),
            source: Some(RebornSkillSourceKind::User),
            mode: RebornSkillActivationMode::ExplicitMention,
            bundle_id: None,
        };
        let error = runtime
            .read_skill_execution_asset(
                &conversation,
                &result.plan,
                &inactive,
                "references/policy.md",
            )
            .await
            .expect_err("inactive skill asset should be denied");
        assert!(
            matches!(error, RebornRuntimeError::SkillExecution(ref reason) if reason.contains("not active")),
            "unexpected inactive asset error: {error}"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn execute_skill_message_returns_unavailable_with_configured_skill_context_source() {
        let root = tempfile::tempdir().expect("tempdir");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "should not reach model".to_string(),
            requests: Arc::clone(&requests),
        });
        let skill_context_source: Arc<dyn HostSkillContextSource> =
            Arc::new(StaticSkillContextSource::new(Vec::new()));
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-skill-execution-unavailable-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-skill-execution-unavailable-tenant".to_string(),
            agent_id: "runtime-skill-execution-unavailable-agent".to_string(),
            source_binding_id: "runtime-skill-execution-unavailable-source".to_string(),
            reply_target_binding_id: "runtime-skill-execution-unavailable-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_skill_context_source(skill_context_source)
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let error = runtime
            .execute_skill_message(&conversation, "please use system-helper")
            .await
            .expect_err("custom skill context source should not expose execution adapter");

        assert!(matches!(
            error,
            RebornRuntimeError::SkillExecutionUnavailable
        ));
        assert!(
            requests
                .lock()
                .expect("recording gateway requests lock poisoned")
                .is_empty(),
            "skill execution should fail before model dispatch when the execution adapter is unavailable"
        );

        let plan = RebornSkillExecutionPlan {
            activations: Vec::new(),
            rewritten_message: String::new(),
            feedback: Vec::new(),
            active_bundles: Vec::new(),
            asset_scope_bundles: Vec::new(),
        };
        let activation = RebornSkillActivation {
            name: "system-helper".to_string(),
            source: Some(RebornSkillSourceKind::System),
            mode: RebornSkillActivationMode::ExplicitMention,
            bundle_id: None,
        };
        let error = runtime
            .read_skill_execution_asset(&conversation, &plan, &activation, "references/policy.md")
            .await
            .expect_err("custom skill context source should not expose asset reader");

        assert!(matches!(
            error,
            RebornRuntimeError::SkillExecutionUnavailable
        ));

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_fails_closed_for_invalid_filesystem_skill_before_model_call() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("skills/bad-helper")).expect("bad skill dir");
        std::fs::write(
            storage_root.join("skills/bad-helper/SKILL.md"),
            skill_md(
                "different-name",
                "bad helper description",
                "BAD_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write bad skill");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "should not reach model".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-bad-skill-owner", storage_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-bad-skill-tenant".to_string(),
            agent_id: "runtime-bad-skill-agent".to_string(),
            source_binding_id: "runtime-bad-skill-source".to_string(),
            reply_target_binding_id: "runtime-bad-skill-reply".to_string(),
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
            runtime.send_user_message(&conversation, "review this"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_ne!(reply.status, TurnStatus::Completed);
        assert!(
            requests
                .lock()
                .expect("recording gateway requests lock poisoned")
                .is_empty(),
            "invalid filesystem skill should fail before model dispatch"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_maps_workspace_to_configured_root() {
        let root = tempfile::tempdir().expect("tempdir");
        let workspace_root = tempfile::tempdir().expect("workspace tempdir");
        std::fs::write(
            workspace_root.path().join("workspace-sentinel.txt"),
            "visible through /workspace",
        )
        .expect("write sentinel");
        let gateway = Arc::new(WorkspaceListingGateway::default());
        let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-workspace-owner", root.path().join("local-dev"))
                .with_local_dev_workspace_root(workspace_root.path().to_path_buf())
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-workspace-tenant".to_string(),
            agent_id: "runtime-workspace-agent".to_string(),
            source_binding_id: "runtime-workspace-source".to_string(),
            reply_target_binding_id: "runtime-workspace-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway_for_runtime);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            Duration::from_secs(3),
            runtime.send_user_message(&conversation, "list workspace"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("workspace ok"));
        let request_count = {
            let requests = gateway
                .requests
                .lock()
                .expect("workspace gateway requests lock poisoned");
            requests.len()
        };
        assert_eq!(
            request_count, 2,
            "workspace listing should require initial request plus tool-result follow-up"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_webui_bundle_reuses_thread_and_turn_facades() {
        let root = tempfile::tempdir().expect("tempdir");
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-webui-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-tenant".to_string(),
            agent_id: "runtime-webui-agent".to_string(),
            source_binding_id: "runtime-webui-source".to_string(),
            reply_target_binding_id: "runtime-webui-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        });

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let runtime_turn_coordinator = runtime.webui_turn_coordinator();
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");

        let _api = bundle.api.clone();
        assert!(Arc::ptr_eq(
            &runtime_turn_coordinator,
            &runtime.webui_turn_coordinator()
        ));
        assert_eq!(bundle.readiness, runtime.services().readiness);
        assert_eq!(bundle.readiness.state, RebornReadinessState::DevOnly);

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
