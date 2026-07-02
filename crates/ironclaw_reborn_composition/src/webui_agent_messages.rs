//! Default WebUI "Agent messages" thread and triggered-run notifications.
//!
//! Agent-initiated messages (routine/trigger failures, runs blocked on the
//! user, and trigger results whose configured delivery target is the WebUI)
//! land in one deterministic, discoverable per-user thread instead of being
//! buried inside individual trigger-run threads.
//!
//! Two pieces live here:
//!
//! - [`WebUiAgentMessenger`] — ensures the per-user "Agent messages" thread
//!   exists and appends finalized assistant messages to it through the
//!   canonical [`SessionThreadService`] transcript boundary (never through
//!   WebUI projections).
//! - [`WebUiTriggeredRunNotifier`] — a [`TriggerFireSettlementObserver`] that
//!   watches each submitted trigger run to a terminal or blocked state and
//!   posts to the default thread when the WebUI owns delivery:
//!   - run failed → always post (failures are otherwise only visible inside
//!     the buried run history);
//!   - run blocked on approval/auth or run completed → post only when the
//!     resolved delivery candidate is the WebUI default-thread target, or
//!     when no delivery target is configured at all (fallback, so results
//!     are never silently lost). External targets (Slack, Telegram) stay
//!     owned by their product delivery drivers and outbound policy.
//!
//! Delivery-target *selection* stays under `ironclaw_outbound`
//! ([`resolve_triggered_final_reply_candidate`]); this module only consumes
//! the resolved candidate. Nothing here performs external egress, so the
//! fail-closed rule for external sends is preserved.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ThreadId, UserId};
use ironclaw_outbound::{
    CommunicationPreferenceRepository, OutboundError, TriggerCommunicationContext, TriggerFireSlot,
    TriggerOriginRef, TriggerSourceKind, resolve_triggered_final_reply_candidate,
};
use ironclaw_threads::{
    AppendFinalizedAssistantMessageRequest, EnsureThreadRequest,
    FinalizedAssistantMessageByRunRequest, MessageContent, SessionThreadError,
    SessionThreadService, ThreadScope,
};
use ironclaw_triggers::{
    TriggerAcceptedFireSettlement, TriggerFire, TriggerFireSettlementObserver,
};
use ironclaw_turns::{GetRunStateRequest, TurnActor, TurnCoordinator, TurnRunId, TurnScope};
use tokio::sync::Semaphore;

use crate::webui_outbound_targets::webui_default_thread_reply_target_binding_ref;

pub(crate) const WEBUI_AGENT_MESSAGES_THREAD_TITLE: &str = "Agent messages";

/// Deterministic per-scope thread id for the default "Agent messages" thread.
///
/// Derived with UUIDv5 over the thread scope so every notification for the
/// same tenant/agent/user lands in the same thread and the thread can be
/// found again without any lookup table.
pub(crate) fn webui_agent_messages_thread_id(scope: &ThreadScope) -> ThreadId {
    let owner = scope
        .owner_user_id
        .as_ref()
        .map(|user| user.as_str())
        .unwrap_or("");
    let project = scope
        .project_id
        .as_ref()
        .map(|project| project.as_str())
        .unwrap_or("");
    let key = format!(
        "ironclaw:webui-agent-messages:{}:{}:{}:{}",
        scope.tenant_id.as_str(),
        scope.agent_id.as_str(),
        project,
        owner,
    );
    let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, key.as_bytes());
    // safety: a hyphenated UUID string is always a valid scope id.
    ThreadId::new(id.to_string()).expect("uuid v5 string is a valid thread id")
}

/// Posts agent-initiated messages into the default WebUI thread.
pub(crate) struct WebUiAgentMessenger {
    thread_service: Arc<dyn SessionThreadService>,
    /// Must match the agent id used when trigger prompts are recorded (see
    /// `TriggeredRunDeliveryDriver::fallback_agent_id`) so the default thread
    /// lives in the same scope the WebUI lists threads under.
    fallback_agent_id: AgentId,
}

impl WebUiAgentMessenger {
    pub(crate) fn new(
        thread_service: Arc<dyn SessionThreadService>,
        fallback_agent_id: AgentId,
    ) -> Self {
        Self {
            thread_service,
            fallback_agent_id,
        }
    }

    fn thread_scope_for(&self, scope: &TurnScope, fallback_user: &UserId) -> ThreadScope {
        ThreadScope {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope
                .agent_id
                .clone()
                .unwrap_or_else(|| self.fallback_agent_id.clone()),
            project_id: scope.project_id.clone(),
            owner_user_id: Some(
                scope
                    .explicit_owner_user_id()
                    .cloned()
                    .unwrap_or_else(|| fallback_user.clone()),
            ),
            mission_id: None,
        }
    }

    /// Ensure the default thread exists and append one finalized assistant
    /// message to it. Returns the default thread id.
    pub(crate) async fn post_agent_message(
        &self,
        scope: &TurnScope,
        fallback_user: &UserId,
        turn_run_id: String,
        text: String,
    ) -> Result<ThreadId, SessionThreadError> {
        let thread_scope = self.thread_scope_for(scope, fallback_user);
        let thread_id = webui_agent_messages_thread_id(&thread_scope);
        let actor_id = thread_scope
            .owner_user_id
            .as_ref()
            .map(|user| user.as_str().to_string())
            .unwrap_or_else(|| fallback_user.as_str().to_string());
        self.thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: actor_id,
                title: Some(WEBUI_AGENT_MESSAGES_THREAD_TITLE.to_string()),
                metadata_json: None,
            })
            .await?;
        self.thread_service
            .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
                scope: thread_scope,
                thread_id: thread_id.clone(),
                turn_run_id,
                content: MessageContent::text(text),
            })
            .await?;
        Ok(thread_id)
    }
}

/// Coarse probe of a triggered run's progress, narrowed so tests can drive
/// the notifier without a full `TurnCoordinator`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TriggeredRunOutcomeProbe {
    Pending,
    Completed,
    Failed { category: Option<String> },
    NeedsAttention { reason: &'static str },
    Cancelled,
    Unavailable,
}

#[async_trait]
pub(crate) trait TriggeredRunOutcomeSource: Send + Sync {
    async fn run_outcome(&self, scope: &TurnScope, run_id: TurnRunId) -> TriggeredRunOutcomeProbe;
}

/// Production probe over the composed [`TurnCoordinator`].
pub(crate) struct TurnCoordinatorRunOutcomeSource {
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

impl TurnCoordinatorRunOutcomeSource {
    pub(crate) fn new(turn_coordinator: Arc<dyn TurnCoordinator>) -> Self {
        Self { turn_coordinator }
    }
}

#[async_trait]
impl TriggeredRunOutcomeSource for TurnCoordinatorRunOutcomeSource {
    async fn run_outcome(&self, scope: &TurnScope, run_id: TurnRunId) -> TriggeredRunOutcomeProbe {
        let state = match self
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
        {
            Ok(state) => state,
            Err(error) => {
                tracing::debug!(
                    target = "ironclaw::reborn::webui_agent_messages",
                    %run_id,
                    %error,
                    "triggered run outcome probe failed"
                );
                return TriggeredRunOutcomeProbe::Unavailable;
            }
        };
        use ironclaw_turns::TurnStatus;
        match state.status {
            TurnStatus::Completed => TriggeredRunOutcomeProbe::Completed,
            TurnStatus::Failed => TriggeredRunOutcomeProbe::Failed {
                category: state.failure.map(|failure| failure.into_category()),
            },
            TurnStatus::RecoveryRequired => TriggeredRunOutcomeProbe::Failed {
                category: Some("recovery_required".to_string()),
            },
            TurnStatus::Cancelled => TriggeredRunOutcomeProbe::Cancelled,
            TurnStatus::BlockedApproval => TriggeredRunOutcomeProbe::NeedsAttention {
                reason: "a tool approval",
            },
            TurnStatus::BlockedAuth => TriggeredRunOutcomeProbe::NeedsAttention {
                reason: "an account authorization",
            },
            TurnStatus::Queued
            | TurnStatus::Running
            | TurnStatus::BlockedResource
            | TurnStatus::BlockedDependentRun
            | TurnStatus::BlockedExternalTool
            | TurnStatus::CancelRequested => TriggeredRunOutcomeProbe::Pending,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct WebUiTriggeredRunNotifierSettings {
    pub(crate) poll_interval: Duration,
    pub(crate) max_wait: Duration,
    pub(crate) max_concurrent_watches: usize,
}

impl Default for WebUiTriggeredRunNotifierSettings {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(500),
            max_wait: Duration::from_secs(30 * 60),
            max_concurrent_watches: 16,
        }
    }
}

/// Watches submitted trigger runs and posts WebUI default-thread
/// notifications. See the module docs for the delivery-ownership rules.
pub(crate) struct WebUiTriggeredRunNotifier {
    messenger: Arc<WebUiAgentMessenger>,
    thread_service: Arc<dyn SessionThreadService>,
    run_outcomes: Arc<dyn TriggeredRunOutcomeSource>,
    communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
    settings: WebUiTriggeredRunNotifierSettings,
    watch_permits: Arc<Semaphore>,
}

impl WebUiTriggeredRunNotifier {
    pub(crate) fn new(
        messenger: Arc<WebUiAgentMessenger>,
        thread_service: Arc<dyn SessionThreadService>,
        run_outcomes: Arc<dyn TriggeredRunOutcomeSource>,
        communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
        settings: WebUiTriggeredRunNotifierSettings,
    ) -> Self {
        let watch_permits = Arc::new(Semaphore::new(settings.max_concurrent_watches.max(1)));
        Self {
            messenger,
            thread_service,
            run_outcomes,
            communication_preferences,
            settings,
            watch_permits,
        }
    }
}

#[async_trait]
impl TriggerFireSettlementObserver for WebUiTriggeredRunNotifier {
    async fn on_accepted_fire_settled(&self, event: TriggerAcceptedFireSettlement) {
        let Ok(permit) = Arc::clone(&self.watch_permits).try_acquire_owned() else {
            tracing::warn!(
                target = "ironclaw::reborn::webui_agent_messages",
                run_id = %event.run_id,
                "webui triggered-run watch skipped: watch queue full"
            );
            return;
        };
        let watcher = WatchTask {
            messenger: Arc::clone(&self.messenger),
            thread_service: Arc::clone(&self.thread_service),
            run_outcomes: Arc::clone(&self.run_outcomes),
            communication_preferences: Arc::clone(&self.communication_preferences),
            settings: self.settings,
        };
        tokio::spawn(async move {
            let _permit = permit;
            watcher.watch_run(event).await;
        });
    }
}

struct WatchTask {
    messenger: Arc<WebUiAgentMessenger>,
    thread_service: Arc<dyn SessionThreadService>,
    run_outcomes: Arc<dyn TriggeredRunOutcomeSource>,
    communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
    settings: WebUiTriggeredRunNotifierSettings,
}

impl WatchTask {
    async fn watch_run(&self, event: TriggerAcceptedFireSettlement) {
        let TriggerAcceptedFireSettlement {
            fire,
            run_id,
            turn_scope: scope,
        } = event;
        let deadline = tokio::time::Instant::now() + self.settings.max_wait;
        loop {
            match self.run_outcomes.run_outcome(&scope, run_id).await {
                TriggeredRunOutcomeProbe::Pending | TriggeredRunOutcomeProbe::Unavailable => {
                    if tokio::time::Instant::now() >= deadline {
                        tracing::debug!(
                            target = "ironclaw::reborn::webui_agent_messages",
                            %run_id,
                            "webui triggered-run watch gave up: run not settled before max_wait"
                        );
                        return;
                    }
                    tokio::time::sleep(self.settings.poll_interval).await;
                }
                TriggeredRunOutcomeProbe::Cancelled => return,
                TriggeredRunOutcomeProbe::Failed { category } => {
                    let detail = category
                        .filter(|category| !category.is_empty())
                        .map(|category| format!(" ({category})"))
                        .unwrap_or_default();
                    let text = format!(
                        "Automation \u{201c}{}\u{201d} failed{}. \
                         Open the run thread for details, or check the automation's recent runs.",
                        trigger_label(&fire),
                        detail,
                    );
                    self.post(&fire, run_id, &scope, text).await;
                    return;
                }
                TriggeredRunOutcomeProbe::NeedsAttention { reason } => {
                    if self.webui_owns_delivery(&fire, &scope).await {
                        let text = format!(
                            "Automation \u{201c}{}\u{201d} is waiting on {reason}. \
                             Open the run thread to continue it.",
                            trigger_label(&fire),
                        );
                        self.post(&fire, run_id, &scope, text).await;
                    }
                    return;
                }
                TriggeredRunOutcomeProbe::Completed => {
                    if self.webui_owns_delivery(&fire, &scope).await {
                        let text = match self.final_reply_text(&fire, run_id, &scope).await {
                            Some(reply) => format!(
                                "Automation \u{201c}{}\u{201d} finished:\n\n{reply}",
                                trigger_label(&fire),
                            ),
                            None => format!(
                                "Automation \u{201c}{}\u{201d} finished. \
                                 Open the run thread for the full result.",
                                trigger_label(&fire),
                            ),
                        };
                        self.post(&fire, run_id, &scope, text).await;
                    }
                    return;
                }
            }
        }
    }

    /// Whether the WebUI default thread owns delivery for this trigger run:
    /// either the resolved candidate is the WebUI target, or no delivery
    /// target is configured at all (WebUI is the fallback so nothing is
    /// silently lost). Any external candidate belongs to its product
    /// delivery driver.
    async fn webui_owns_delivery(&self, fire: &TriggerFire, scope: &TurnScope) -> bool {
        let Ok(trigger) = trigger_communication_context(fire) else {
            return false;
        };
        let actor = TurnActor::new(fire.creator_user_id.clone());
        match resolve_triggered_final_reply_candidate(
            self.communication_preferences.as_ref(),
            scope,
            &actor,
            &trigger,
        )
        .await
        {
            Ok(target) => target == webui_default_thread_reply_target_binding_ref(),
            Err(OutboundError::PreferenceTargetMissing { .. }) => true,
            Err(error) => {
                tracing::debug!(
                    target = "ironclaw::reborn::webui_agent_messages",
                    %error,
                    "webui triggered-run delivery resolution failed; skipping notification"
                );
                false
            }
        }
    }

    async fn final_reply_text(
        &self,
        fire: &TriggerFire,
        run_id: TurnRunId,
        scope: &TurnScope,
    ) -> Option<String> {
        let thread_scope = self
            .messenger
            .thread_scope_for(scope, &fire.creator_user_id);
        let message = self
            .thread_service
            .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                scope: thread_scope,
                thread_id: scope.thread_id.clone(),
                turn_run_id: run_id.to_string(),
            })
            .await
            .ok()
            .flatten()?;
        message.content.filter(|content| !content.is_empty())
    }

    async fn post(&self, fire: &TriggerFire, run_id: TurnRunId, scope: &TurnScope, text: String) {
        if let Err(error) = self
            .messenger
            .post_agent_message(scope, &fire.creator_user_id, run_id.to_string(), text)
            .await
        {
            tracing::warn!(
                target = "ironclaw::reborn::webui_agent_messages",
                %run_id,
                %error,
                "failed to post webui agent message"
            );
        }
    }
}

fn trigger_label(fire: &TriggerFire) -> String {
    fire.identity.trigger_id().to_string()
}

fn trigger_communication_context(
    fire: &TriggerFire,
) -> Result<TriggerCommunicationContext, String> {
    let trigger_origin_ref = TriggerOriginRef::new(fire.identity.trigger_id().to_string())
        .map_err(|error| format!("invalid trigger origin ref: {error}"))?;
    let fire_slot = TriggerFireSlot::new(fire.identity.fire_slot().to_rfc3339())
        .map_err(|error| format!("invalid fire slot: {error}"))?;
    Ok(TriggerCommunicationContext {
        trigger_origin_ref,
        trigger_source_kind: TriggerSourceKind::Schedule,
        fire_slot,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Instant;

    use chrono::Utc;
    use ironclaw_host_api::TenantId;
    use ironclaw_outbound::{
        CommunicationModality, CommunicationPreferenceRecord, DeliveryDefaultScope,
        InMemoryOutboundStateStore,
    };
    use ironclaw_threads::{InMemorySessionThreadService, ThreadHistoryRequest};
    use ironclaw_triggers::{TriggerFireIdentity, TriggerId};
    use ironclaw_turns::ReplyTargetBindingRef;

    use super::*;
    use crate::webui_outbound_targets::WEBUI_DEFAULT_THREAD_REPLY_TARGET_BINDING_REF;

    const TENANT: &str = "webui-agent-messages-tenant";
    const AGENT: &str = "webui-agent-messages-agent";
    const USER: &str = "webui-agent-messages-user";

    struct ScriptedOutcomeSource {
        outcomes: Mutex<Vec<TriggeredRunOutcomeProbe>>,
    }

    impl ScriptedOutcomeSource {
        fn new(outcomes: Vec<TriggeredRunOutcomeProbe>) -> Self {
            Self {
                outcomes: Mutex::new(outcomes),
            }
        }
    }

    #[async_trait]
    impl TriggeredRunOutcomeSource for ScriptedOutcomeSource {
        async fn run_outcome(
            &self,
            _scope: &TurnScope,
            _run_id: TurnRunId,
        ) -> TriggeredRunOutcomeProbe {
            let mut outcomes = self.outcomes.lock().expect("lock outcomes");
            if outcomes.len() > 1 {
                outcomes.remove(0)
            } else {
                outcomes
                    .first()
                    .cloned()
                    .unwrap_or(TriggeredRunOutcomeProbe::Pending)
            }
        }
    }

    fn tenant() -> TenantId {
        TenantId::new(TENANT).expect("tenant")
    }

    fn agent() -> AgentId {
        AgentId::new(AGENT).expect("agent")
    }

    fn user() -> UserId {
        UserId::new(USER).expect("user")
    }

    fn run_thread_id(run_id: TurnRunId) -> ThreadId {
        ThreadId::new(format!("run-thread-{run_id}")).expect("thread id")
    }

    fn settlement(run_id: TurnRunId) -> TriggerAcceptedFireSettlement {
        let fire = TriggerFire {
            identity: TriggerFireIdentity::new(tenant(), TriggerId::new(), Utc::now()),
            creator_user_id: user(),
            agent_id: Some(agent()),
            project_id: None,
            prompt: "webui agent messages test prompt".to_string(),
        };
        let scope = TurnScope::new_with_owner(
            tenant(),
            Some(agent()),
            None,
            run_thread_id(run_id),
            Some(user()),
        );
        TriggerAcceptedFireSettlement {
            fire,
            run_id,
            turn_scope: scope,
        }
    }

    fn default_thread_scope() -> ThreadScope {
        ThreadScope {
            tenant_id: tenant(),
            agent_id: agent(),
            project_id: None,
            owner_user_id: Some(user()),
            mission_id: None,
        }
    }

    fn notifier(
        thread_service: Arc<InMemorySessionThreadService>,
        outcomes: Vec<TriggeredRunOutcomeProbe>,
        preferences: Arc<InMemoryOutboundStateStore>,
    ) -> WebUiTriggeredRunNotifier {
        let messenger = Arc::new(WebUiAgentMessenger::new(
            Arc::clone(&thread_service) as Arc<dyn SessionThreadService>,
            agent(),
        ));
        WebUiTriggeredRunNotifier::new(
            messenger,
            thread_service as Arc<dyn SessionThreadService>,
            Arc::new(ScriptedOutcomeSource::new(outcomes)),
            preferences as Arc<dyn CommunicationPreferenceRepository>,
            WebUiTriggeredRunNotifierSettings {
                poll_interval: Duration::from_millis(5),
                max_wait: Duration::from_millis(500),
                max_concurrent_watches: 4,
            },
        )
    }

    async fn default_thread_messages(thread_service: &InMemorySessionThreadService) -> Vec<String> {
        let thread_id = webui_agent_messages_thread_id(&default_thread_scope());
        match thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: default_thread_scope(),
                thread_id,
            })
            .await
        {
            Ok(history) => history
                .messages
                .into_iter()
                .filter_map(|message| message.content)
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    async fn wait_for_messages(
        thread_service: &InMemorySessionThreadService,
        expected: usize,
    ) -> Vec<String> {
        let stop = Instant::now() + Duration::from_secs(5);
        loop {
            let messages = default_thread_messages(thread_service).await;
            if messages.len() >= expected {
                return messages;
            }
            if Instant::now() >= stop {
                panic!("expected {expected} default-thread message(s) within 5s, got {messages:?}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn seed_default_preference(store: &InMemoryOutboundStateStore, target: &str) {
        store
            .put_communication_preference(CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::personal(tenant(), user()),
                trigger_origin_ref: None,
                final_reply_target: Some(
                    ReplyTargetBindingRef::new(target).expect("valid reply target"),
                ),
                progress_target: None,
                approval_prompt_target: None,
                auth_prompt_target: None,
                default_modality: Some(CommunicationModality::Text),
                updated_at: Utc::now(),
                updated_by: user(),
            })
            .await
            .expect("seed preference");
    }

    #[tokio::test]
    async fn failed_run_posts_into_the_default_agent_messages_thread() {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let preferences = Arc::new(InMemoryOutboundStateStore::default());
        // A Slack default is configured; failures must still land in WebUI.
        seed_default_preference(&preferences, "reply:slack-default").await;
        let notifier = notifier(
            Arc::clone(&thread_service),
            vec![
                TriggeredRunOutcomeProbe::Pending,
                TriggeredRunOutcomeProbe::Failed {
                    category: Some("model_error".to_string()),
                },
            ],
            preferences,
        );

        notifier
            .on_accepted_fire_settled(settlement(TurnRunId::new()))
            .await;

        let messages = wait_for_messages(&thread_service, 1).await;
        assert!(
            messages[0].contains("failed") && messages[0].contains("model_error"),
            "failure message must carry the sanitized category: {messages:?}"
        );
    }

    #[tokio::test]
    async fn completed_run_with_webui_target_posts_the_final_reply() {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let preferences = Arc::new(InMemoryOutboundStateStore::default());
        seed_default_preference(&preferences, WEBUI_DEFAULT_THREAD_REPLY_TARGET_BINDING_REF).await;
        let run_id = TurnRunId::new();
        let event = settlement(run_id);

        // Record the run's final reply in its own (buried) run thread.
        let run_scope = ThreadScope {
            tenant_id: tenant(),
            agent_id: agent(),
            project_id: None,
            owner_user_id: Some(user()),
            mission_id: None,
        };
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: run_scope.clone(),
                thread_id: Some(run_thread_id(run_id)),
                created_by_actor_id: USER.to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("ensure run thread");
        thread_service
            .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
                scope: run_scope,
                thread_id: run_thread_id(run_id),
                turn_run_id: run_id.to_string(),
                content: MessageContent::text("the daily report is ready"),
            })
            .await
            .expect("record final reply");

        let notifier = notifier(
            Arc::clone(&thread_service),
            vec![TriggeredRunOutcomeProbe::Completed],
            preferences,
        );
        notifier.on_accepted_fire_settled(event).await;

        let messages = wait_for_messages(&thread_service, 1).await;
        assert!(
            messages[0].contains("the daily report is ready"),
            "webui-target completion must repost the final reply: {messages:?}"
        );
    }

    #[tokio::test]
    async fn completed_run_with_external_target_stays_out_of_the_default_thread() {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let preferences = Arc::new(InMemoryOutboundStateStore::default());
        seed_default_preference(&preferences, "reply:slack-default").await;
        let notifier = notifier(
            Arc::clone(&thread_service),
            vec![TriggeredRunOutcomeProbe::Completed],
            preferences,
        );

        notifier
            .on_accepted_fire_settled(settlement(TurnRunId::new()))
            .await;

        // Give the watch task time to (incorrectly) post.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            default_thread_messages(&thread_service).await.is_empty(),
            "external delivery targets are owned by their product drivers"
        );
    }

    #[tokio::test]
    async fn completed_run_without_any_configured_target_falls_back_to_the_default_thread() {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let preferences = Arc::new(InMemoryOutboundStateStore::default());
        let notifier = notifier(
            Arc::clone(&thread_service),
            vec![TriggeredRunOutcomeProbe::Completed],
            preferences,
        );

        notifier
            .on_accepted_fire_settled(settlement(TurnRunId::new()))
            .await;

        let messages = wait_for_messages(&thread_service, 1).await;
        assert!(
            messages[0].contains("finished"),
            "unconfigured delivery must fall back to the default thread: {messages:?}"
        );
    }

    #[tokio::test]
    async fn blocked_run_with_webui_target_posts_a_needs_attention_message() {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let preferences = Arc::new(InMemoryOutboundStateStore::default());
        seed_default_preference(&preferences, WEBUI_DEFAULT_THREAD_REPLY_TARGET_BINDING_REF).await;
        let notifier = notifier(
            Arc::clone(&thread_service),
            vec![TriggeredRunOutcomeProbe::NeedsAttention {
                reason: "a tool approval",
            }],
            preferences,
        );

        notifier
            .on_accepted_fire_settled(settlement(TurnRunId::new()))
            .await;

        let messages = wait_for_messages(&thread_service, 1).await;
        assert!(
            messages[0].contains("waiting on a tool approval"),
            "blocked runs must surface the challenge: {messages:?}"
        );
    }

    #[tokio::test]
    async fn per_trigger_override_to_webui_routes_that_trigger_into_the_default_thread() {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let preferences = Arc::new(InMemoryOutboundStateStore::default());
        seed_default_preference(&preferences, "reply:slack-default").await;
        let run_id = TurnRunId::new();
        let event = settlement(run_id);

        // Per-trigger override: this trigger delivers to the WebUI even though
        // the scoped default is Slack.
        preferences
            .put_communication_preference(CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::personal(tenant(), user()),
                trigger_origin_ref: Some(
                    TriggerOriginRef::new(event.fire.identity.trigger_id().to_string())
                        .expect("valid trigger origin"),
                ),
                final_reply_target: Some(
                    ReplyTargetBindingRef::new(WEBUI_DEFAULT_THREAD_REPLY_TARGET_BINDING_REF)
                        .expect("valid reply target"),
                ),
                progress_target: None,
                approval_prompt_target: None,
                auth_prompt_target: None,
                default_modality: None,
                updated_at: Utc::now(),
                updated_by: user(),
            })
            .await
            .expect("seed override");

        let notifier = notifier(
            Arc::clone(&thread_service),
            vec![TriggeredRunOutcomeProbe::Completed],
            preferences,
        );
        notifier.on_accepted_fire_settled(event).await;

        let messages = wait_for_messages(&thread_service, 1).await;
        assert!(
            messages[0].contains("finished"),
            "per-trigger webui override must deliver to the default thread: {messages:?}"
        );
    }

    #[tokio::test]
    async fn messenger_reuses_one_deterministic_thread_across_messages() {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let messenger = WebUiAgentMessenger::new(
            Arc::clone(&thread_service) as Arc<dyn SessionThreadService>,
            agent(),
        );
        let scope = TurnScope::new_with_owner(
            tenant(),
            Some(agent()),
            None,
            run_thread_id(TurnRunId::new()),
            Some(user()),
        );

        let first = messenger
            .post_agent_message(&scope, &user(), TurnRunId::new().to_string(), "one".into())
            .await
            .expect("post first");
        let second = messenger
            .post_agent_message(&scope, &user(), TurnRunId::new().to_string(), "two".into())
            .await
            .expect("post second");

        assert_eq!(first, second, "both messages must land in the same thread");
        let messages = default_thread_messages(&thread_service).await;
        assert_eq!(messages, vec!["one".to_string(), "two".to_string()]);
    }
}
