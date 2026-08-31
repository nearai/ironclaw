//! Production implementations of the publication service's ports, over the
//! turn kernel and the thread service, plus the process-journal hook that
//! resumes publications on a run's terminal commit.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::reply::{
    REPLY_DISPLAY_PREVIEW_MAX_BYTES, REPLY_DISPLAY_TEXT_MAX_BYTES, ReplyAttentionKind,
    ReplyAudience, ReplyDisplayPreview, ReplyDisplayText, ReplyDocument, ReplyTarget,
};
use ironclaw_host_api::turn::{TurnActor, TurnGateRef, TurnRunId, TurnScope, TurnStatus};
use ironclaw_processes::{
    ProcessJournalCommit, ProcessJournalCommitObserver, ProcessKind, ProcessLifecycleStatus,
};
use ironclaw_product_contracts::prompt_source::{
    ApprovalPromptContextSource, BlockedAuthPromptRequest, BlockedAuthPromptSource,
};
use ironclaw_threads::{FinalizedAssistantMessageByRunRequest, SessionThreadService, ThreadScope};
use ironclaw_turns::{CancelRunRequest, GetRunStateRequest, TurnCoordinator, TurnExecutionOutcome};

use super::{
    ReplyAttentionEnricher, ReplyPublicationError, ReplyPublicationService, ReplyStopRequester,
    TerminalReplyFactSource,
};
use crate::reply_projection::TerminalReplyFacts;
use crate::run_delivery::prompts;

const LOG_TARGET: &str = "ironclaw::reborn::reply_publication";

/// Terminal facts from the two durable owners: the run's committed state
/// (turn kernel) and its finalized transcript row (thread service).
pub struct KernelTerminalReplyFacts {
    turn_coordinator: Arc<dyn TurnCoordinator>,
    thread_service: Arc<dyn SessionThreadService>,
}

impl KernelTerminalReplyFacts {
    pub fn new(
        turn_coordinator: Arc<dyn TurnCoordinator>,
        thread_service: Arc<dyn SessionThreadService>,
    ) -> Self {
        Self {
            turn_coordinator,
            thread_service,
        }
    }
}

#[async_trait]
impl TerminalReplyFactSource for KernelTerminalReplyFacts {
    async fn terminal_reply_facts(
        &self,
        scope: &TurnScope,
        actor: &TurnActor,
        run_id: TurnRunId,
    ) -> Result<TerminalReplyFacts, ReplyPublicationError> {
        let state = self
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
            .map_err(|error| ReplyPublicationError::TerminalFactsUnavailable {
                reason: format!("run state unavailable: {error}"),
            })?;
        let mut facts = TerminalReplyFacts {
            actor: state.actor.clone().or_else(|| Some(actor.clone())),
            status: state.status,
            nothing_to_report: state.execution_outcome
                == Some(TurnExecutionOutcome::NothingToReport),
            answer: None,
            attachments: Vec::new(),
            failure_summary: state.failure.as_ref().and_then(sanitized_failure_text),
        };
        // Only a completion that produced an answer has a transcript row to
        // read; a nothing-to-report completion publishes an empty terminal
        // revision so every target still closes.
        if state.status != TurnStatus::Completed || facts.nothing_to_report {
            return Ok(facts);
        }
        let Some(agent_id) = scope.agent_id.clone() else {
            return Err(ReplyPublicationError::TerminalFactsUnavailable {
                reason: "a completed run without an agent scope has no transcript to read"
                    .to_string(),
            });
        };
        let thread_scope = ThreadScope {
            tenant_id: scope.tenant_id.clone(),
            agent_id,
            project_id: scope.project_id.clone(),
            owner_user_id: Some(actor.user_id.clone()),
            mission_id: None,
        };
        let message = self
            .thread_service
            .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                scope: thread_scope,
                thread_id: scope.thread_id.clone(),
                turn_run_id: run_id.to_string(),
            })
            .await
            .map_err(|error| ReplyPublicationError::TerminalFactsUnavailable {
                reason: format!("finalized assistant message unavailable: {error}"),
            })?;
        if let Some(message) = message {
            facts.answer = message.content;
            facts.attachments = message.attachments;
        }
        Ok(facts)
    }
}

/// The kernel's failure record serializes to `{category, detail}`; the
/// category is a stable classification and the detail is already sanitized
/// by the kernel. Read through the wire shape (the fields are private).
fn sanitized_failure_text(failure: &ironclaw_host_api::turn::SanitizedFailure) -> Option<String> {
    let value = serde_json::to_value(failure).ok()?;
    let category = value.get("category")?.as_str()?.trim().to_string();
    let detail = value
        .get("detail")
        .and_then(|detail| detail.as_str())
        .map(str::trim)
        .filter(|detail| !detail.is_empty());
    if category.is_empty() {
        return None;
    }
    Some(match detail {
        Some(detail) => format!("{category}: {detail}"),
        None => category,
    })
}

/// `StoppedByUser` from a sink becomes an ordinary cancel through the turn
/// kernel — the same path the web app's stop button takes.
pub struct TurnCoordinatorStopRequester {
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

impl TurnCoordinatorStopRequester {
    pub fn new(turn_coordinator: Arc<dyn TurnCoordinator>) -> Self {
        Self { turn_coordinator }
    }
}

#[async_trait]
impl ReplyStopRequester for TurnCoordinatorStopRequester {
    async fn request_stop(&self, scope: &TurnScope, actor: &TurnActor, run_id: TurnRunId) {
        let idempotency_key = match ironclaw_turns::IdempotencyKey::new(format!(
            "reply-stop-{run_id}"
        )) {
            Ok(key) => key,
            Err(error) => {
                tracing::debug!(target: LOG_TARGET, %run_id, %error, "reply stop idempotency key invalid");
                return;
            }
        };
        if let Err(error) = self
            .turn_coordinator
            .cancel_run(CancelRunRequest {
                scope: scope.clone(),
                actor: actor.clone(),
                run_id,
                reason: ironclaw_turns::SanitizedCancelReason::UserRequested,
                idempotency_key,
            })
            .await
        {
            tracing::debug!(target: LOG_TARGET, %run_id, %error, "reply stop request was not accepted by the turn kernel");
        }
    }
}

/// Fills the attention facet from the gate's durable record: the approval
/// context (what, why) for an approval gate; the auth challenge (headline,
/// body, authorization URL) for an auth gate. The publisher applies audience
/// disclosure after this, so a URL placed here never reaches a shared room.
pub struct GateAttentionEnricher {
    approval_context: Option<Arc<dyn ApprovalPromptContextSource>>,
    blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

impl GateAttentionEnricher {
    pub fn new(
        approval_context: Option<Arc<dyn ApprovalPromptContextSource>>,
        blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        Self {
            approval_context,
            blocked_auth_prompts,
            turn_coordinator,
        }
    }
}

#[async_trait]
impl ReplyAttentionEnricher for GateAttentionEnricher {
    async fn enrich(&self, target: &ReplyTarget, document: &mut ReplyDocument) {
        let Some(attention) = document.attention.as_mut() else {
            return;
        };
        let Some(gate_ref) = attention
            .gate_ref
            .as_ref()
            .and_then(|gate_ref| TurnGateRef::new(gate_ref.as_str()).ok())
        else {
            return;
        };
        let direct_message = target.audience == ReplyAudience::Private;
        match attention.kind {
            ReplyAttentionKind::Approval => {
                let context = match &self.approval_context {
                    Some(source) => {
                        source
                            .approval_prompt_context(
                                &gate_ref,
                                &target.actor.user_id,
                                &target.scope,
                            )
                            .await
                    }
                    None => None,
                };
                let view =
                    prompts::approval_gate_prompt_view(target.run_id, &gate_ref, context.as_ref());
                if let Some(headline) = display_text(&view.headline) {
                    attention.headline = headline;
                }
                attention.body = display_preview(&prompts::gate_prompt_text(&view, direct_message));
            }
            ReplyAttentionKind::Auth => {
                let Some(source) = &self.blocked_auth_prompts else {
                    return;
                };
                let state = match self
                    .turn_coordinator
                    .get_run_state(GetRunStateRequest {
                        scope: target.scope.clone(),
                        run_id: target.run_id,
                    })
                    .await
                {
                    Ok(state) => state,
                    Err(error) => {
                        tracing::debug!(target: LOG_TARGET, run_id = %target.run_id, %error, "auth attention enrichment: run state unavailable");
                        return;
                    }
                };
                let view = match source
                    .auth_prompt_for_blocked_run(BlockedAuthPromptRequest {
                        fallback_owner_user_id: &target.actor.user_id,
                        scope: &target.scope,
                        run_id: target.run_id,
                        gate_ref: &gate_ref,
                        invocation_id: None,
                        body: "Authenticate to continue this run.".to_string(),
                        credential_requirements: &state.credential_requirements,
                    })
                    .await
                {
                    Ok(view) => view,
                    Err(error) => {
                        tracing::debug!(target: LOG_TARGET, run_id = %target.run_id, %error, "auth attention enrichment: prompt unavailable");
                        return;
                    }
                };
                if let Some(headline) = display_text(&view.headline) {
                    attention.headline = headline;
                }
                if prompts::auth_prompt_is_serviceable(&view) {
                    attention.body = display_preview(&prompts::actionable_auth_prompt_body(&view));
                    // Bearer material: only a private target may carry it,
                    // and disclosure enforces that after this step too.
                    attention.action_url = if direct_message {
                        view.authorization_url.as_deref().and_then(display_text)
                    } else {
                        None
                    };
                } else {
                    attention.body =
                        display_preview(prompts::unserviceable_auth_prompt_message(Some(&view)));
                    attention.action_url = None;
                }
            }
            ReplyAttentionKind::Resource => {}
        }
    }
}

fn display_text(value: &str) -> Option<ReplyDisplayText> {
    let stripped: String = value
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    let mut end = stripped.len().min(REPLY_DISPLAY_TEXT_MAX_BYTES);
    while end > 0 && !stripped.is_char_boundary(end) {
        end -= 1;
    }
    ReplyDisplayText::new(&stripped[..end]).ok() // safety: `end` walked back to a char boundary above.
}

fn display_preview(value: &str) -> Option<ReplyDisplayPreview> {
    let stripped: String = value
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    let mut end = stripped.len().min(REPLY_DISPLAY_PREVIEW_MAX_BYTES);
    while end > 0 && !stripped.is_char_boundary(end) {
        end -= 1;
    }
    ReplyDisplayPreview::new(&stripped[..end]).ok() // safety: `end` walked back to a char boundary above.
}

/// Resumes publications from the durable side: every terminal commit of a
/// top-level user run tells the service the run is over. On the node that
/// ran the run this is a fast path to the terminal facts; on any other node
/// it is how an orphaned publication (a crashed publisher, a lapsed lease)
/// gets a worker again.
pub struct ReplyPublicationCommitObserver {
    service: Arc<ReplyPublicationService>,
}

impl ReplyPublicationCommitObserver {
    pub fn new(service: Arc<ReplyPublicationService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl ProcessJournalCommitObserver for ReplyPublicationCommitObserver {
    fn process_observer_id(&self) -> &'static str {
        "reply-publication-commit-observer-v1"
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        let snapshot = &commit.state;
        if snapshot.process_kind != ProcessKind::AgentTurn
            || snapshot.parent_process_id.is_some()
            || !snapshot.status.is_terminal()
            || snapshot.status == ProcessLifecycleStatus::RecoveryRequired
        {
            return Ok(());
        }
        let (Some(thread_id), Some(agent_id)) = (
            snapshot.scope.thread_id.clone(),
            snapshot.scope.agent_id.clone(),
        ) else {
            return Ok(());
        };
        let scope = TurnScope::new(
            snapshot.scope.tenant_id.clone(),
            Some(agent_id),
            snapshot.scope.project_id.clone(),
            thread_id,
        );
        let run_id = TurnRunId::from_uuid(snapshot.process_id.as_uuid());
        // Never block the journal on publication work; the service is
        // idempotent, so a lost wake-up is caught by the next signal.
        let service = Arc::clone(&self.service);
        tokio::spawn(async move {
            service.run_terminal(&scope, run_id).await;
        });
        Ok(())
    }
}
