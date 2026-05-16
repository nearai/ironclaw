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

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use thiserror::Error;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};

use crate::runtime_admin::{RebornAdminClient, RebornAdminScope};
use crate::runtime_input::{ConversationIdentity, RebornIdentityConfig};
use ironclaw_reborn::driver_registry::{DriverKind, DriverRegistry, DriverRequirements};
use ironclaw_reborn::loop_driver_host::{RebornLoopDriverHostFactory, TextOnlyLoopHostConfig};
use ironclaw_reborn::loop_exit_applier::ThreadCheckpointLoopExitEvidencePort;
use ironclaw_reborn::text_loop_driver::TextOnlyModelReplyDriver;
use ironclaw_reborn::turn_runner::{
    TurnRunnerWakeReceiver, TurnRunnerWorker, TurnRunnerWorkerConfig,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService,
    MessageContent, MessageKind, MessageStatus, SessionThreadService, ThreadHistoryRequest,
    ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, AgentLoopDriver, DefaultTurnCoordinator, GetRunStateRequest,
    IdempotencyKey, InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore,
    InMemoryTurnStateStore, LoopExitApplier, ReplyTargetBindingRef, RunProfileId,
    RunProfileRequest, RunProfileVersion, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnRunId, TurnScope, TurnStatus,
    run_profile::{
        CapabilitySurfaceProfileId, CheckpointSchemaId, InMemoryRunProfileRegistry,
        InMemoryRunProfileResolver, RunProfileDefinition,
    },
};

use crate::{
    RebornBuildError, RebornCompositionProfile, RebornServices, build_reborn_services,
};
use crate::runtime_input::{RebornRuntimeApiVersion, RebornRuntimeInput, TurnRunnerSettings};

#[cfg(feature = "root-llm-provider")]
use crate::runtime_input::RebornLlmConfig;

/// Stable identifier for a Reborn CLI conversation. Wraps a `ThreadId`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConversationId(pub ThreadId);

/// Per-conversation tenant/agent/owner/project resolution recorded at
/// conversation creation. Stored so `send_user_message` can construct the
/// right `TurnScope` for each conversation rather than reusing the
/// runtime-wide default.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ConversationContext {
    tenant: TenantId,
    agent: AgentId,
    owner: UserId,
    project: Option<ProjectId>,
    thread_scope: ThreadScope,
}

/// Final-form assistant reply read back from the session thread service after
/// a `send_user_message` completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantReply {
    pub conversation: ConversationId,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub text: Option<String>,
}

/// Per-message options for `RebornRuntime::send_user_message`.
///
/// Today only `requested_run_profile` is consumed (forwarded into
/// `SubmitTurnRequest`); `harness_override` is recorded and logged but
/// has no effect until the harness substrate lands (epic #3036).
#[derive(Debug, Clone, Default)]
pub struct SendMessageOptions {
    /// Requested run-profile identifier. Maps onto
    /// `RunProfileResolver` keys — drives which loop driver runs the
    /// turn. `None` means "use the runtime's implicit default profile".
    pub requested_run_profile: Option<String>,
    /// Per-message harness override. `Some` records the operator's
    /// intent for this turn; takes effect once the harness substrate
    /// lands (epic #3036).
    pub harness_override: Option<crate::runtime_input::RebornHarnessId>,
}

impl SendMessageOptions {
    pub fn with_requested_run_profile(mut self, profile: impl Into<String>) -> Self {
        self.requested_run_profile = Some(profile.into());
        self
    }

    pub fn with_harness(
        mut self,
        harness: crate::runtime_input::RebornHarnessId,
    ) -> Self {
        self.harness_override = Some(harness);
        self
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
    #[error("invalid scope or identifier: {reason}")]
    InvalidArgument { reason: String },
    #[error("incompatible RebornRuntimeInput.api_version: {0}")]
    IncompatibleApiVersion(String),
    #[error("reborn runtime admin surface: {0}")]
    Admin(String),
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

/// Custom run-profile id used by the Reborn standalone runtime. Distinct from
/// the v1 builtin `interactive_default` so registrations don't collide.
/// Only lowercase ASCII letters, digits, `_`, `-`, `:` are accepted by the
/// bounded-loop-string validator.
pub(crate) fn reborn_runtime_profile_id() -> RunProfileId {
    RunProfileId::new("reborn_text_only").expect("static profile id is valid")
}

/// Capability surface profile id paired with the text-only driver. The
/// text-only driver does not consume capabilities, so this is informational.
pub(crate) fn reborn_runtime_capability_surface_id() -> CapabilitySurfaceProfileId {
    CapabilitySurfaceProfileId::new("reborn_text_only_no_tools")
        .expect("static capability surface id is valid")
}

/// Started, running Reborn agent runtime.
///
/// `RebornRuntime` exposes two narrow surfaces:
///
/// 1. **Agent surface** — `new_conversation`, `new_conversation_for`,
///    `send_user_message`, `shutdown`. Channels, REPLs, and remote
///    inbound handlers use these.
/// 2. **Admin surface** — `admin(scope)` returns a [`RebornAdminClient`]
///    for privileged operations (blueprint apply, harness install /
///    activate). Construction requires a [`RebornAdminScope`] that is
///    sealed against accidental escalation.
///
/// Downstream code never reaches into the substrate or worker machinery:
/// it talks to the runtime through task-level methods.
pub struct RebornRuntime {
    services: RebornServices,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    thread_service: Arc<InMemorySessionThreadService>,
    /// Default identity used for `new_conversation()` (no override).
    /// Per-conversation overrides are recorded in `conversations` below.
    default_identity: RebornIdentityConfig,
    /// Per-conversation context — maps each `ConversationId` to the
    /// resolved tenant/agent/owner/project that conversation was
    /// created with. The lookup happens on every `send_user_message`.
    conversations: tokio::sync::RwLock<
        std::collections::HashMap<ThreadId, ConversationContext>,
    >,
    worker_handle: JoinHandle<()>,
    worker_cancel: CancellationToken,
    poll_settings: PollSettings,
}

#[derive(Debug, Clone)]
struct PollSettings {
    interval: Duration,
    max_total: Duration,
}

impl Default for PollSettings {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(100),
            max_total: Duration::from_secs(180),
        }
    }
}

impl RebornRuntime {
    /// Snapshot of the substrate facades produced by `build_reborn_services`.
    /// Exposed for diagnostics / readiness reporting; **not** for traffic.
    pub fn services(&self) -> &RebornServices {
        &self.services
    }

    /// Obtain the privileged admin surface backed by the supplied scope.
    ///
    /// `scope` is sealed — construction is restricted to the CLI binary
    /// via [`RebornAdminScope::for_cli_admin`]. Every admin method returns
    /// `RebornAdminError::NotYetWired` today; the substrate that makes
    /// them live lands in epic #3036.
    pub fn admin(&self, scope: RebornAdminScope) -> RebornAdminClient {
        RebornAdminClient::new(scope)
    }

    /// Default identity this runtime was built with.
    pub fn default_identity(&self) -> &RebornIdentityConfig {
        &self.default_identity
    }

    /// Create a fresh conversation bound to the runtime's default identity.
    pub async fn new_conversation(&self) -> Result<ConversationId, RebornRuntimeError> {
        self.new_conversation_for(ConversationIdentity::default()).await
    }

    /// Create a fresh conversation with a per-conversation tenant / agent /
    /// owner / project override. Any field left `None` on `override_` falls
    /// back to the runtime's `default_identity`.
    ///
    /// The thread is materialized inside the session thread service so
    /// `accept_inbound_message` does not error on the first send, and the
    /// per-conversation identity is recorded so subsequent
    /// `send_user_message` calls can construct the right `TurnScope`.
    pub async fn new_conversation_for(
        &self,
        override_: ConversationIdentity,
    ) -> Result<ConversationId, RebornRuntimeError> {
        let tenant = override_.tenant.unwrap_or_else(|| self.default_identity.tenant.clone());
        let agent = override_
            .agent
            .unwrap_or_else(|| self.default_identity.default_agent.clone());
        let owner = override_
            .owner
            .unwrap_or_else(|| self.default_identity.default_owner.clone());
        let project = override_
            .project
            .or_else(|| self.default_identity.default_project.clone());

        let thread_scope = ThreadScope {
            tenant_id: tenant.clone(),
            agent_id: agent.clone(),
            project_id: project.clone(),
            owner_user_id: Some(owner.clone()),
            mission_id: None,
        };
        let thread_id = ThreadId::new(format!("reborn-conv-{}", Uuid::new_v4()))
            .map_err(|reason| RebornRuntimeError::InvalidArgument {
                reason: reason.to_string(),
            })?;
        self.thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: owner.as_str().to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .map_err(|error| RebornRuntimeError::ThreadService(error.to_string()))?;
        let context = ConversationContext {
            tenant,
            agent,
            owner,
            project,
            thread_scope,
        };
        self.conversations.write().await.insert(thread_id.clone(), context);
        Ok(ConversationId(thread_id))
    }

    /// Submit a user message and wait for the run to reach a terminal
    /// state. Convenience wrapper over
    /// `send_user_message_with_options` with default options.
    pub async fn send_user_message(
        &self,
        conversation: &ConversationId,
        text: &str,
    ) -> Result<AssistantReply, RebornRuntimeError> {
        self.send_user_message_with_options(conversation, text, SendMessageOptions::default())
            .await
    }

    /// Submit a user message with explicit per-turn options. The options
    /// flow into `SubmitTurnRequest` so the run-profile resolver / driver
    /// registry pick the right driver for the turn.
    ///
    /// Without an LLM gateway wired in (i.e. when this crate is built
    /// without the `root-llm-provider` feature or `RebornLlmConfig` is not
    /// provided), the run will fail and the returned reply will surface
    /// that failure via `status = Failed` (or `RecoveryRequired`) and
    /// `text = None`.
    pub async fn send_user_message_with_options(
        &self,
        conversation: &ConversationId,
        text: &str,
        options: SendMessageOptions,
    ) -> Result<AssistantReply, RebornRuntimeError> {
        if self.worker_handle.is_finished() {
            return Err(RebornRuntimeError::WorkerStopped);
        }
        let context = self
            .conversations
            .read()
            .await
            .get(&conversation.0)
            .cloned()
            .ok_or_else(|| RebornRuntimeError::InvalidArgument {
                reason: format!(
                    "unknown ConversationId {} -- was new_conversation() called?",
                    conversation.0.as_str()
                ),
            })?;

        // Per-message harness override is recorded today; it takes effect
        // once the harness substrate from epic #3036 lands.
        if let Some(harness) = &options.harness_override {
            tracing::warn!(
                harness_id = %harness,
                thread_id = %conversation.0,
                "SendMessageOptions.harness_override is recorded but not yet wired (epic #3036)"
            );
        }

        let requested_run_profile = match options.requested_run_profile {
            Some(profile) => Some(
                RunProfileRequest::new(profile)
                    .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?,
            ),
            None => None,
        };

        let scope = TurnScope::new(
            context.tenant.clone(),
            Some(context.agent.clone()),
            context.project.clone(),
            conversation.0.clone(),
        );
        let accepted = self
            .thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: context.thread_scope.clone(),
                thread_id: conversation.0.clone(),
                actor_id: context.owner.as_str().to_string(),
                source_binding_id: Some("reborn-cli".to_string()),
                reply_target_binding_id: Some("reborn-cli".to_string()),
                external_event_id: Some(format!("reborn-cli:{}", Uuid::new_v4())),
                content: MessageContent::text(text.to_string()),
            })
            .await
            .map_err(|error| RebornRuntimeError::ThreadService(error.to_string()))?;

        let accepted_message_ref =
            AcceptedMessageRef::new(format!("msg:{}", accepted.message_id))
                .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?;
        let source_binding_ref = SourceBindingRef::new("reborn-cli")
            .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?;
        let reply_target_binding_ref = ReplyTargetBindingRef::new("reborn-cli")
            .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?;
        let idempotency_key = IdempotencyKey::new(format!("reborn-cli-{}", Uuid::new_v4()))
            .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?;

        let response = self
            .turn_coordinator
            .submit_turn(SubmitTurnRequest {
                scope: scope.clone(),
                actor: TurnActor::new(context.owner.clone()),
                accepted_message_ref: accepted_message_ref.clone(),
                source_binding_ref,
                reply_target_binding_ref,
                requested_run_profile,
                idempotency_key,
                received_at: Utc::now(),
            })
            .await?;

        let SubmitTurnResponse::Accepted { run_id, .. } = response;

        let terminal_status = self.wait_for_terminal(&scope, run_id).await?;
        let assistant_text = self
            .read_latest_assistant_text(&context.thread_scope, &conversation.0, run_id)
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
        let _ = self.worker_handle.await;
        Ok(())
    }

    async fn wait_for_terminal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
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
            if state.status.is_terminal()
                || matches!(state.status, TurnStatus::RecoveryRequired)
            {
                // RecoveryRequired isn't "terminal" in the durable state
                // machine (a future recovery worker could resume it), but for
                // the standalone CLI it is end-of-line: there is no recovery
                // worker in this composition. Surface it like Failed.
                return Ok(state.status);
            }
            if start.elapsed() > self.poll_settings.max_total {
                return Err(RebornRuntimeError::RunTimeout {
                    timeout: self.poll_settings.max_total,
                });
            }
            tokio::time::sleep(self.poll_settings.interval).await;
        }
    }

    async fn read_latest_assistant_text(
        &self,
        thread_scope: &ThreadScope,
        thread_id: &ThreadId,
        run_id: TurnRunId,
    ) -> Result<Option<String>, RebornRuntimeError> {
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope.clone(),
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
        api_version,
        services: services_input,
        identity,
        policy,
        drivers,
        harness,
        #[cfg(feature = "root-llm-provider")]
        llm,
        runner,
    } = input;

    api_version
        .compatible_with(RebornRuntimeApiVersion::current())
        .map_err(|error| RebornRuntimeError::IncompatibleApiVersion(error.to_string()))?;

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

    // Today's composition wires only the text-only driver end-to-end.
    // The planned-driver variant is a recognized choice (operator-facing
    // config validation accepts it) but selecting it as the default at
    // boot is a clear NotYetWired error rather than a silent fallback.
    if drivers.default == crate::runtime_input::RebornDriverChoice::Planned {
        return Err(RebornRuntimeError::InvalidArgument {
            reason: "RebornDriverConfig.default = Planned is recognized but not yet \
                     wired end-to-end in build_reborn_runtime; track epic #3036 sub-issue \
                     'harness composition over planned driver'"
                .to_string(),
        });
    }

    // Harness selection is forward-compat-only today: epic #3036 ships the
    // HarnessRepo + HarnessActivationService + capability-surface filter
    // that make this take effect. Record the operator's intent in the boot
    // log so audit traces show *why* the runtime is starting without an
    // overlay even when a harness is requested.
    if let Some(selection) = &harness {
        tracing::warn!(
            harness_id = %selection.harness_id,
            "RebornHarnessSelection requested but harness substrate is not yet wired (epic #3036); proceeding without overlay"
        );
    }

    tracing::debug!(
        api_version = %api_version,
        tenant = %identity.tenant,
        agent = %identity.default_agent,
        owner = %identity.default_owner,
        deployment_mode = %policy.deployment_mode.as_str(),
        default_profile = ?policy.default_profile,
        approval_policy = ?policy.default_approval_policy,
        driver_default = %drivers.default,
        driver_additional = ?drivers.additional,
        "build_reborn_runtime: composing assembled runtime"
    );

    let _ = identity.default_project.as_ref();
    let _ = (&policy, &drivers); // policy/drivers are recorded; values
    // surface in the boot log above. Field-level enforcement is downstream
    // work in epic #3036 (RuntimePolicyRepo + per-turn resolver).

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

    // Custom run-profile resolver pointing at the text-only driver.
    let text_only_descriptor = TextOnlyModelReplyDriver::default().descriptor();
    let mut registry = InMemoryRunProfileRegistry::with_builtin_profiles();
    registry
        .register(RunProfileDefinition::interactive_like(
            reborn_runtime_profile_id(),
            text_only_descriptor.clone(),
            text_only_descriptor
                .checkpoint_schema_id
                .clone()
                .unwrap_or_else(|| {
                    CheckpointSchemaId::new("reborn_text_only_checkpoint")
                        .expect("static checkpoint id is valid")
                }),
            text_only_descriptor
                .checkpoint_schema_version
                .unwrap_or(RunProfileVersion::new(1)),
            reborn_runtime_capability_surface_id(),
        ))
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("could not register reborn run profile: {error}"),
        })?;
    let resolver = InMemoryRunProfileResolver::new_with_implicit_default(
        registry,
        reborn_runtime_profile_id(),
    );

    let turn_coordinator: Arc<dyn TurnCoordinator> = Arc::new(
        DefaultTurnCoordinator::new(Arc::clone(&turn_state_store) as Arc<_>)
            .with_run_profile_resolver(Arc::new(resolver)),
    );

    // Default thread scope: built from the runtime's default identity.
    // Per-conversation overrides are recorded in `RebornRuntime.conversations`
    // and shadowed in their own `ThreadScope`.
    let default_thread_scope = ThreadScope {
        tenant_id: identity.tenant.clone(),
        agent_id: identity.default_agent.clone(),
        project_id: identity.default_project.clone(),
        owner_user_id: Some(identity.default_owner.clone()),
        mission_id: None,
    };

    // `owner_id` from substrate is kept for compatibility but is no
    // longer used to mint the actor identity — that comes from
    // `RebornIdentityConfig` now.
    let _ = owner_id;

    // Driver registry + worker — these are gated on the model gateway being
    // available (i.e. the `root-llm-provider` feature + LLM config).
    let (worker_cancel, worker_handle) = build_and_spawn_worker(
        runner,
        Arc::clone(&turn_state_store),
        Arc::clone(&checkpoint_state_store) as Arc<_>,
        Arc::clone(&loop_checkpoint_store) as Arc<_>,
        Arc::clone(&thread_service),
        default_thread_scope,
        text_only_descriptor.clone(),
        #[cfg(feature = "root-llm-provider")]
        llm,
    )?;

    Ok(RebornRuntime {
        services,
        turn_coordinator,
        thread_service,
        default_identity: identity,
        conversations: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        worker_handle,
        worker_cancel,
        poll_settings: PollSettings::default(),
    })
}

#[allow(clippy::too_many_arguments)]
fn build_and_spawn_worker(
    runner: TurnRunnerSettings,
    turn_state_store: Arc<InMemoryTurnStateStore>,
    checkpoint_state_store: Arc<dyn ironclaw_turns::CheckpointStateStore>,
    loop_checkpoint_store: Arc<dyn ironclaw_turns::LoopCheckpointStore>,
    thread_service: Arc<InMemorySessionThreadService>,
    thread_scope: ThreadScope,
    text_only_descriptor: ironclaw_turns::run_profile::AgentLoopDriverDescriptor,
    #[cfg(feature = "root-llm-provider")] llm: Option<RebornLlmConfig>,
) -> Result<(CancellationToken, JoinHandle<()>), RebornRuntimeError> {
    use ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink;

    // Driver registry — registers the text-only driver under its descriptor.
    let mut registry = DriverRegistry::new();
    let text_only_driver = Arc::new(TextOnlyModelReplyDriver::default());
    registry
        .register_driver(
            text_only_driver,
            DriverRequirements::all_optional(),
            DriverKind::Production,
        )
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("could not register text-only driver: {error:?}"),
        })?;
    let _ = text_only_descriptor; // descriptor used implicitly through the driver

    let driver_registry = Arc::new(registry);

    // Build the model gateway adapter, if available. Always return *some*
    // gateway (stub if needed) so the worker boots even when the operator
    // hasn't configured an LLM yet; the failure surfaces at first
    // `send_user_message` instead of at startup. This keeps `ironclaw-reborn
    // run` ergonomic when an operator just wants to confirm the binary boots.
    #[cfg(feature = "root-llm-provider")]
    let model_gateway = match llm {
        Some(cfg) => build_llm_gateway(cfg)?,
        None => build_stub_gateway(),
    };
    #[cfg(not(feature = "root-llm-provider"))]
    let model_gateway = build_stub_gateway();

    let milestone_sink: Arc<dyn ironclaw_turns::run_profile::LoopHostMilestoneSink> =
        Arc::new(InMemoryLoopHostMilestoneSink::default());

    let host_factory = RebornLoopDriverHostFactory::new(
        Arc::clone(&thread_service),
        thread_scope.clone(),
        model_gateway,
        Arc::clone(&checkpoint_state_store),
        Arc::clone(&turn_state_store) as Arc<dyn ironclaw_turns::TurnStateStore>,
        Arc::clone(&loop_checkpoint_store),
        milestone_sink,
        TextOnlyLoopHostConfig::default(),
    );
    let host_factory: Arc<dyn ironclaw_reborn::turn_runner::HostFactory> = Arc::new(host_factory);

    let evidence_port = Arc::new(ThreadCheckpointLoopExitEvidencePort::new(
        Arc::clone(&thread_service),
        Arc::clone(&turn_state_store) as Arc<dyn ironclaw_turns::TurnStateStore>,
        Arc::clone(&loop_checkpoint_store),
    ));
    let loop_exit_applier = Arc::new(LoopExitApplier::new(
        Arc::clone(&turn_state_store) as Arc<dyn ironclaw_turns::runner::TurnRunTransitionPort>,
        evidence_port,
    ));

    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig {
            heartbeat_interval: runner.heartbeat_interval,
            poll_interval: runner.poll_interval,
            scope_filter: None,
        },
        Arc::clone(&turn_state_store) as Arc<dyn ironclaw_turns::runner::TurnRunTransitionPort>,
        loop_exit_applier,
        driver_registry,
        host_factory,
        wake_receiver,
    );
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move {
        worker.run(cancel_clone).await;
    });
    Ok((cancel, handle))
}

#[cfg(feature = "root-llm-provider")]
fn build_llm_gateway(
    cfg: RebornLlmConfig,
) -> Result<
    Arc<dyn ironclaw_loop_support::HostManagedModelGateway>,
    RebornRuntimeError,
> {
    use ironclaw_llm::{ProviderProtocol, RegistryProviderConfig, config::CacheRetention};
    use ironclaw_reborn::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
    use ironclaw_turns::run_profile::ModelProfileId;

    let protocol = match cfg.protocol.as_str() {
        "openai_completions" | "openai" => ProviderProtocol::OpenAiCompletions,
        "anthropic" => ProviderProtocol::Anthropic,
        "ollama" => ProviderProtocol::Ollama,
        "deepseek" => ProviderProtocol::DeepSeek,
        "gemini" => ProviderProtocol::Gemini,
        "openrouter" => ProviderProtocol::OpenRouter,
        "github_copilot" => ProviderProtocol::GithubCopilot,
        other => {
            return Err(RebornRuntimeError::LlmProvider(format!(
                "unsupported llm protocol: {other}"
            )));
        }
    };
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
    let provider = ironclaw_llm::create_registry_provider(&registry_config, cfg.request_timeout_secs)
        .map_err(|error| RebornRuntimeError::LlmProvider(error.to_string()))?;

    let model_profile_id = ModelProfileId::new("interactive_model").expect("static id");
    let policy = LlmModelProfilePolicy::new()
        .allow_model_profile(model_profile_id, Some(cfg.model.clone()));
    let gateway = LlmProviderModelGateway::new(provider, policy);
    Ok(Arc::new(gateway))
}

fn build_stub_gateway()
-> Arc<dyn ironclaw_loop_support::HostManagedModelGateway> {
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
