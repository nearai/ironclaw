use ironclaw_product_contracts::prompt_source::BlockedAuthPromptRequest;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use crate::is_approval_gate_ref;
use async_trait::async_trait;
use futures::{StreamExt, stream};
use ironclaw_approvals::ApprovalRequestStorePort;
use ironclaw_extension_contracts::auth_prompt::AuthPromptContextView;
use ironclaw_host_api::ids::{InvocationId, UserId};
use ironclaw_host_api::product_adapter::{
    ProductAdapterError, ProductSurfaceRejectionKind, RedactedString,
};
use ironclaw_host_api::turn::{
    ModelInvalidOutputDetailReason, SanitizedFailure, TurnGateRef, TurnRunId, TurnScope,
    TurnStatus, TurnThreadOwner,
};
use ironclaw_loop_contracts::{
    SystemInferenceIdentity, SystemInferencePort, SystemInferenceRequest, SystemInferenceTaskId,
    SystemPromptId, SystemPromptSource, SystemTaskKind, sanitize_model_visible_text,
};
use ironclaw_product_contracts::approval_prompt::{
    approval_prompt_context_for_request, approval_prompt_lookup_scope,
    approval_request_id_from_gate_ref,
};
use ironclaw_product_contracts::outbound::{
    ApprovalPromptContextView, GatePromptView, ProductGateKind, ProductOutboundPayload,
    ProductProjectionItem, ProductProjectionState,
};
use ironclaw_threads::{FinalizedAssistantMessageByRunRequest, SessionThreadService, ThreadScope};
use ironclaw_turns::{
    GetRunStateRequest, TurnBlockedGateKind, TurnCoordinator, TurnError, TurnEventKind,
    TurnEventProjectionCursor, TurnEventProjectionError, TurnEventProjectionRequest,
    TurnEventProjectionSource, TurnEventReducerService, TurnLifecycleEvent,
};
use tokio::sync::{Mutex, OnceCell, Semaphore};

use ironclaw_auth::product_prompt::{AuthChallengeProvider, auth_prompt_view_for_blocked_auth};
use ironclaw_host_api::failure::categories::CHECKPOINT_REJECTED_CATEGORY;
use ironclaw_host_api::failure::summary::{
    checkpoint_rejection_host_explanation_from_detail, pinned_failure_summary_for_category,
    reborn_failure_summary_for_category_and_detail,
};

pub(super) const WEBUI_TURN_EVENT_PAGE_LIMIT: usize = 256;
const FAILURE_EXPLANATION_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);
const FAILURE_EXPLANATION_MAX_BYTES: usize = 512;
const FAILURE_EXPLANATION_MAX_INPUT_TOKENS: u64 = 512;
const FAILURE_EXPLANATION_CACHE_CAPACITY: usize = 1000;
const FAILURE_EXPLANATION_MAX_CONCURRENT_MODEL_CALLS: usize = 4;

pub(super) struct TurnEventPayload {
    pub(super) cursor: TurnEventProjectionCursor,
    pub(super) payload: ProductOutboundPayload,
}

#[derive(Debug, Clone)]
pub struct FailureExplanationInput {
    pub failure_category: String,
    pub fallback_summary: String,
    /// Model-visible, secret-scrubbed raw cause carried from the failure
    /// record (e.g. a provider HTTP status line or a missing schema-ref path).
    /// Unlike `fallback_summary`, this is the original error text so the
    /// explainer is given the real facts rather than only the coarse category.
    /// Producers scrub secret VALUES before populating it; the prompt builder
    /// re-runs [`sanitize_model_visible_text`] as defense in depth.
    pub detail: Option<String>,
}

#[async_trait]
pub trait FailureExplanationProvider: Send + Sync {
    async fn explain_failure(&self, input: FailureExplanationInput) -> Option<String>;
}

#[derive(Debug, Default)]
pub struct NoopFailureExplanationProvider;

pub(super) struct TurnEventDrain {
    pub(super) next_cursor: Option<TurnEventProjectionCursor>,
    pub(super) payloads: Vec<TurnEventPayload>,
}

#[derive(Clone, Default)]
pub(super) enum TurnEventBridge {
    #[default]
    Disabled,
    Enabled {
        service: Arc<TurnEventReducerService<dyn TurnEventProjectionSource>>,
        coordinator: Arc<dyn TurnCoordinator>,
        // arch-exempt: optional_arc, approval prompt enrichment is absent in minimal projection graphs, plan #4539
        approval_requests: Option<Arc<dyn ApprovalRequestStorePort>>,
        failure_explainer: Arc<dyn FailureExplanationProvider>,
        failure_explanation_cache: Arc<Mutex<FailureExplanationCache>>,
    },
}

pub struct ModelFailureExplanationProvider {
    system_inference: Arc<dyn Fn() -> Arc<dyn SystemInferencePort> + Send + Sync>,
    permits: Arc<Semaphore>,
}

impl ModelFailureExplanationProvider {
    #[cfg(test)]
    pub fn new(system_inference: Arc<dyn SystemInferencePort>) -> Self {
        Self {
            system_inference: Arc::new(move || Arc::clone(&system_inference)),
            permits: Arc::new(Semaphore::new(
                FAILURE_EXPLANATION_MAX_CONCURRENT_MODEL_CALLS,
            )),
        }
    }

    pub fn from_factory(
        system_inference: Arc<dyn Fn() -> Arc<dyn SystemInferencePort> + Send + Sync>,
    ) -> Self {
        Self {
            system_inference,
            permits: Arc::new(Semaphore::new(
                FAILURE_EXPLANATION_MAX_CONCURRENT_MODEL_CALLS,
            )),
        }
    }
}

impl TurnEventBridge {
    pub(super) fn enabled(
        source: Arc<dyn TurnEventProjectionSource>,
        coordinator: Arc<dyn TurnCoordinator>,
        approval_requests: Option<Arc<dyn ApprovalRequestStorePort>>,
    ) -> Self {
        Self::Enabled {
            service: Arc::new(TurnEventReducerService::new(source)),
            coordinator,
            approval_requests,
            failure_explainer: Arc::new(NoopFailureExplanationProvider),
            failure_explanation_cache: Arc::new(Mutex::new(FailureExplanationCache::new(
                FAILURE_EXPLANATION_CACHE_CAPACITY,
            ))),
        }
    }

    pub(super) fn with_approval_requests(
        mut self,
        requests: Option<Arc<dyn ApprovalRequestStorePort>>,
    ) -> Self {
        if let Self::Enabled {
            approval_requests, ..
        } = &mut self
        {
            *approval_requests = requests;
        }
        self
    }

    pub(super) fn with_failure_explainer(
        mut self,
        explainer: Arc<dyn FailureExplanationProvider>,
    ) -> Self {
        if let Self::Enabled {
            failure_explainer, ..
        } = &mut self
        {
            *failure_explainer = explainer;
        }
        self
    }

    pub(super) async fn drain(
        &self,
        caller_user_id: &ironclaw_host_api::ids::UserId,
        scope: &TurnScope,
        after: Option<TurnEventProjectionCursor>,
        auth_challenges: Option<&dyn AuthChallengeProvider>,
        thread_service: Option<&dyn SessionThreadService>,
    ) -> Result<TurnEventDrain, ProductAdapterError> {
        let Self::Enabled {
            service,
            coordinator,
            approval_requests,
            failure_explainer,
            failure_explanation_cache,
        } = self
        else {
            return Ok(TurnEventDrain {
                next_cursor: after,
                payloads: Vec::new(),
            });
        };
        let mut after_cursor = after;
        let mut payloads = Vec::new();
        let mut next_cursor;
        loop {
            let page = match service
                .updates(TurnEventProjectionRequest {
                    scope: scope.clone(),
                    owner_user_id: Some(caller_user_id.clone()),
                    after: after_cursor.clone(),
                    limit: WEBUI_TURN_EVENT_PAGE_LIMIT,
                })
                .await
            {
                Ok(page) => page,
                Err(TurnEventProjectionError::RebaseRequired {
                    requested,
                    earliest,
                }) if requested.scope == earliest.scope => {
                    // The requested cursor sits below the projection's retention
                    // floor, so the events it asked for are gone. The projection
                    // still tells us the earliest replayable cursor; jump the
                    // client forward to it instead of surfacing a retryable error.
                    //
                    // This applies on reconnect too (a non-`None` cursor), not
                    // just first connect. Otherwise the browser auto-reconnects
                    // via `Last-Event-ID` with the same stale cursor, gets the
                    // same rebase rejection, and loops forever — appearing as a
                    // permanently "disconnected" stream. Skipping the
                    // unavoidably-pruned events keeps the stream alive and
                    // self-corrects: the next drain requests `after = earliest`,
                    // which is at/above the floor and drains normally. Any
                    // payloads already collected this drain are returned first so
                    // we never drop events we did read.
                    return Ok(TurnEventDrain {
                        next_cursor: Some(*earliest),
                        payloads,
                    });
                }
                Err(error) => return Err(map_turn_event_projection_error(error)),
            };
            next_cursor = Some(page.next_cursor.clone());
            let context = TurnEventPayloadContext {
                caller_user_id,
                coordinator: coordinator.as_ref(),
                failure_explainer: failure_explainer.as_ref(),
                failure_explanation_cache,
                auth_challenges,
                approval_requests: approval_requests.as_deref(),
                thread_service,
            };
            payloads.extend(turn_event_payloads_for_page(context, page.entries).await?);
            if !page.truncated || after_cursor.as_ref() == Some(&page.next_cursor) {
                break;
            }
            after_cursor = Some(page.next_cursor);
        }
        Ok(TurnEventDrain {
            next_cursor,
            payloads,
        })
    }
}

#[derive(Clone, Copy)]
struct TurnEventPayloadContext<'a> {
    caller_user_id: &'a UserId,
    coordinator: &'a dyn TurnCoordinator,
    failure_explainer: &'a dyn FailureExplanationProvider,
    failure_explanation_cache: &'a Arc<Mutex<FailureExplanationCache>>,
    auth_challenges: Option<&'a dyn AuthChallengeProvider>,
    approval_requests: Option<&'a dyn ApprovalRequestStorePort>,
    thread_service: Option<&'a dyn SessionThreadService>,
}

async fn turn_event_payloads_for_page(
    context: TurnEventPayloadContext<'_>,
    events: Vec<TurnLifecycleEvent>,
) -> Result<Vec<TurnEventPayload>, ProductAdapterError> {
    let futures = events.into_iter().map(|event| {
        let cursor = TurnEventProjectionCursor::for_scope(event.scope.clone(), event.cursor);
        async move {
            turn_event_payloads(context, &event).await.map(|payloads| {
                payloads
                    .into_iter()
                    .map(|payload| TurnEventPayload {
                        cursor: cursor.clone(),
                        payload,
                    })
                    .collect::<Vec<_>>()
            })
        }
    });
    let payloads = stream::iter(futures)
        .buffered(16)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    Ok(payloads.into_iter().flatten().collect())
}

async fn turn_event_payloads(
    context: TurnEventPayloadContext<'_>,
    event: &TurnLifecycleEvent,
) -> Result<Vec<ProductOutboundPayload>, ProductAdapterError> {
    let mut payloads = Vec::new();
    let blocked_prompt = if matches!(event.kind, TurnEventKind::Blocked) {
        blocked_prompt_payload(
            context.caller_user_id,
            context.coordinator,
            context.auth_challenges,
            context.approval_requests,
            event,
        )
        .await?
    } else {
        None
    };
    if projects_run_status(&event.kind) {
        let failure_details = failure_details_for_turn_event(
            context.failure_explainer,
            context.failure_explanation_cache,
            event,
        )
        .await;
        let finalized_reply =
            finalized_reply_projection_item(context.caller_user_id, context.thread_service, event)
                .await?;
        payloads.push(ProductOutboundPayload::ProjectionUpdate {
            state: turn_event_projection_state(
                event,
                failure_details,
                blocked_prompt.as_ref(),
                finalized_reply,
            )?,
        });
    }
    if let Some(prompt) = blocked_prompt {
        payloads.push(prompt);
    }
    Ok(payloads)
}

/// Load the canonical finalized transcript row for a completed run. Embedding
/// it in the same durable turn-event projection state as `completed` gives
/// stream delivery one restart- and replica-safe proof; live text deltas remain
/// presentation updates and cannot satisfy that proof on their own.
async fn finalized_reply_projection_item(
    caller_user_id: &UserId,
    thread_service: Option<&dyn SessionThreadService>,
    event: &TurnLifecycleEvent,
) -> Result<Option<ProductProjectionItem>, ProductAdapterError> {
    if event.status != TurnStatus::Completed || !matches!(event.kind, TurnEventKind::Completed) {
        return Ok(None);
    }
    let Some(thread_service) = thread_service else {
        return Ok(None);
    };
    let Some(agent_id) = event.scope.agent_id.clone() else {
        return Ok(None);
    };
    let owner_user_id = match &event.scope.thread_owner {
        TurnThreadOwner::ActorFallback => Some(caller_user_id.clone()),
        TurnThreadOwner::ExplicitUser { owner_user_id } => Some(owner_user_id.clone()),
        TurnThreadOwner::Ownerless => None,
    };
    let message = thread_service
        .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
            scope: ThreadScope {
                tenant_id: event.scope.tenant_id.clone(),
                agent_id,
                project_id: event.scope.project_id.clone(),
                owner_user_id,
                mission_id: None,
            },
            thread_id: event.scope.thread_id.clone(),
            turn_run_id: event.run_id.to_string(),
        })
        .await
        .map_err(|error| {
            tracing::debug!(
                %error,
                run_id = %event.run_id,
                "finalized assistant message lookup failed during WebUI projection"
            );
            ProductAdapterError::SurfaceTransient {
                reason: RedactedString::new("finalized assistant message lookup failed"),
            }
        })?;
    let Some(message) = message else {
        return Ok(None);
    };
    let Some(body) = message.content else {
        return Ok(None);
    };
    Ok(Some(ProductProjectionItem::Text {
        id: message.message_id.to_string(),
        run_id: Some(event.run_id),
        body,
        finalized: true,
    }))
}

#[async_trait]
impl FailureExplanationProvider for NoopFailureExplanationProvider {
    async fn explain_failure(&self, _input: FailureExplanationInput) -> Option<String> {
        None
    }
}

#[async_trait]
impl FailureExplanationProvider for ModelFailureExplanationProvider {
    async fn explain_failure(&self, input: FailureExplanationInput) -> Option<String> {
        let Ok(_permit) = self.permits.try_acquire() else {
            tracing::debug!(
                failure_category = %input.failure_category,
                "failed run explanation skipped because model explanation capacity is saturated"
            );
            return None;
        };
        let request = match failure_explanation_request(&input) {
            Some(request) => request,
            None => return None,
        };
        let response = match tokio::time::timeout(
            FAILURE_EXPLANATION_TIMEOUT,
            (self.system_inference)().call_system_inference(request),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                tracing::debug!(
                    error = %error,
                    failure_category = %input.failure_category,
                    "failed run explanation model call failed"
                );
                return None;
            }
            Err(_) => {
                tracing::debug!(
                    failure_category = %input.failure_category,
                    "failed run explanation model call timed out"
                );
                return None;
            }
        };
        bounded_failure_explanation(&response.output_text)
    }
}

async fn blocked_prompt_payload(
    caller_user_id: &ironclaw_host_api::ids::UserId,
    coordinator: &dyn TurnCoordinator,
    auth_challenges: Option<&dyn AuthChallengeProvider>,
    approval_requests: Option<&dyn ApprovalRequestStorePort>,
    event: &TurnLifecycleEvent,
) -> Result<Option<ProductOutboundPayload>, ProductAdapterError> {
    let state = match coordinator
        .get_run_state(GetRunStateRequest {
            scope: event.scope.clone(),
            run_id: event.run_id,
        })
        .await
    {
        Ok(state) => state,
        Err(TurnError::ScopeNotFound) => return Ok(None),
        Err(error) => {
            tracing::debug!(
                %error,
                run_id = %event.run_id,
                "turn gate state lookup failed during WebUI projection"
            );
            return Err(ProductAdapterError::SurfaceTransient {
                reason: RedactedString::new("turn gate state lookup failed"),
            });
        }
    };
    if state.status != event.status || state.event_cursor != event.cursor {
        return Ok(None);
    }
    let blocked_invocation_id = event
        .blocked_gate
        .as_ref()
        .and_then(|gate| gate.activity_id)
        .or(state.blocked_activity_id)
        .map(|activity_id| InvocationId::from_uuid(activity_id.as_uuid()));
    let Some(gate_ref) = state.gate_ref.as_ref() else {
        return Ok(None);
    };
    let gate_ref_str = gate_ref.as_str().to_string();
    match event.status {
        TurnStatus::BlockedAuth => {
            let view = auth_prompt_view_for_blocked_auth(BlockedAuthPromptRequest {
                fallback_owner_user_id: event.owner_user_id.as_ref().unwrap_or(caller_user_id),
                scope: &event.scope,
                run_id: event.run_id,
                gate_ref,
                invocation_id: blocked_invocation_id,
                body: event
                    .sanitized_reason
                    .clone()
                    .unwrap_or_else(|| "Authenticate to continue this run.".to_string()),
                credential_requirements: &state.credential_requirements,
            },
            auth_challenges)
            .await?;
            Ok(Some(ProductOutboundPayload::AuthPrompt(view)))
        }
        TurnStatus::BlockedApproval => Ok(Some(
            approval_gate_prompt(
                caller_user_id,
                approval_requests,
                event,
                gate_ref,
                gate_ref_str,
            )
            .await?,
        )),
        TurnStatus::BlockedResource => Ok(Some(gate_prompt(
            event,
            gate_ref_str,
            "Resource unavailable",
            false,
        ))),
        // Non-blocked statuses: no prompt payload. Exhaustive match so a new
        // TurnStatus variant forces a compile error and an explicit decision.
        TurnStatus::Queued
        | TurnStatus::Running
        | TurnStatus::BlockedDependentRun
        // External-tool gates are not user-clickable prompts; the OpenAI
        // Responses surface reads them via its own projection path. No generic
        // gate-prompt payload here.
        | TurnStatus::BlockedExternalTool
        | TurnStatus::RecoveryRequired
        | TurnStatus::CancelRequested
        | TurnStatus::Completed
        | TurnStatus::Cancelled
        | TurnStatus::Failed => Ok(None),
    }
}

async fn approval_gate_prompt(
    caller_user_id: &UserId,
    approval_requests: Option<&dyn ApprovalRequestStorePort>,
    event: &TurnLifecycleEvent,
    gate_ref: &TurnGateRef,
    gate_ref_string: String,
) -> Result<ProductOutboundPayload, ProductAdapterError> {
    let owner_user_id = event.owner_user_id.as_ref().unwrap_or(caller_user_id);
    // Fail closed (#6203 parity): an approval-store outage renders as a
    // transient stream failure, never a contextless-but-actionable prompt.
    let lookup = approval_prompt_lookup(approval_requests, gate_ref, owner_user_id, &event.scope)
        .await
        .map_err(|error| {
            tracing::debug!(
                %error,
                run_id = %event.run_id,
                "approval request lookup failed during gate projection"
            );
            ProductAdapterError::SurfaceTransient {
                reason: RedactedString::new("approval request lookup failed"),
            }
        })?;
    Ok(gate_prompt_with_context(
        event,
        gate_ref_string,
        "Approval required",
        is_approval_gate_ref(gate_ref.as_str()),
        lookup.context,
        lookup.invocation_id,
    ))
}

/// Resolve an approval gate's request details (tool/action/reason) into the
/// rendered context view, by looking it up in the `ApprovalRequestStorePort` by
/// gate ref. Shared by the WebUI gate projection and the Slack approval prompt
/// so both surface the *same* "what is being approved" data from one source.
/// Returns `None` when no store is wired, the gate ref is not an approval ref,
/// the request is missing, or the lookup fails.
///
/// The projection itself lives in
/// `ironclaw_product_contracts::approval_prompt`; this wrapper adds the store
/// read and the documented best-effort degradation.
pub async fn approval_prompt_context_view(
    approval_requests: Option<&dyn ApprovalRequestStorePort>,
    gate_ref: &TurnGateRef,
    owner_user_id: &UserId,
    turn_scope: &TurnScope,
) -> Option<ApprovalPromptContextView> {
    approval_prompt_lookup(approval_requests, gate_ref, owner_user_id, turn_scope)
        .await
        .unwrap_or_default() // silent-ok: documented best-effort enrichment for delivery prompts; the WebUI gate projection path fails transiently instead
        .context
}

#[derive(Debug, Default)]
struct ApprovalPromptLookup {
    context: Option<ApprovalPromptContextView>,
    invocation_id: Option<InvocationId>,
}

async fn approval_prompt_lookup(
    approval_requests: Option<&dyn ApprovalRequestStorePort>,
    gate_ref: &TurnGateRef,
    owner_user_id: &UserId,
    turn_scope: &TurnScope,
) -> Result<ApprovalPromptLookup, ironclaw_approvals::ApprovalStoreError> {
    let (store, request_id) =
        match approval_requests.zip(approval_request_id_from_gate_ref(gate_ref)) {
            Some(value) => value,
            None => return Ok(ApprovalPromptLookup::default()),
        };
    let scope = approval_prompt_lookup_scope(turn_scope, owner_user_id);
    Ok(match store.get(&scope, request_id).await? {
        Some(record) => ApprovalPromptLookup {
            context: approval_prompt_context_for_request(&record.request),
            invocation_id: Some(record.scope.invocation_id),
        },
        None => ApprovalPromptLookup::default(),
    })
}

fn gate_prompt(
    event: &TurnLifecycleEvent,
    gate_ref: String,
    headline: &'static str,
    allow_always: bool,
) -> ProductOutboundPayload {
    gate_prompt_with_context(event, gate_ref, headline, allow_always, None, None)
}

fn gate_prompt_with_context(
    event: &TurnLifecycleEvent,
    gate_ref: String,
    headline: &'static str,
    allow_always: bool,
    approval_context: Option<ApprovalPromptContextView>,
    invocation_id: Option<InvocationId>,
) -> ProductOutboundPayload {
    ProductOutboundPayload::GatePrompt(GatePromptView {
        turn_run_id: event.run_id,
        gate_ref,
        invocation_id,
        headline: headline.to_string(),
        body: event
            .sanitized_reason
            .clone()
            .unwrap_or_else(|| "Resolve this gate to continue the run.".to_string()),
        allow_always,
        approval_context,
    })
}

fn projects_run_status(kind: &TurnEventKind) -> bool {
    matches!(
        kind,
        TurnEventKind::Submitted
            | TurnEventKind::Resumed
            | TurnEventKind::RunnerClaimed
            | TurnEventKind::RecoveryRequired
            | TurnEventKind::Blocked
            | TurnEventKind::CancelRequested
            | TurnEventKind::Cancelled
            | TurnEventKind::Completed
            | TurnEventKind::Failed
    )
}

fn turn_event_projection_state(
    event: &TurnLifecycleEvent,
    failure_details: FailureProjectionDetails,
    blocked_prompt: Option<&ProductOutboundPayload>,
    finalized_reply: Option<ProductProjectionItem>,
) -> Result<ProductProjectionState, ProductAdapterError> {
    let mut items = vec![ProductProjectionItem::RunStatus {
        run_id: event.run_id,
        status: turn_status_wire(event.status).to_string(),
        failure_category: failure_details.category,
        failure_summary: failure_details.summary,
        retryable: event.retryable,
    }];
    if let Some(item) = gate_projection_item(event, blocked_prompt)? {
        items.push(item);
    }
    if let Some(item) = finalized_reply {
        items.push(item);
    }
    ProductProjectionState::new(event.scope.thread_id.to_string(), items)
}

#[derive(Debug, Clone, Default)]
struct GateProjectionPromptContext {
    invocation_id: Option<InvocationId>,
    headline: Option<String>,
    body: Option<String>,
    allow_always: Option<bool>,
    auth_context: Option<AuthPromptContextView>,
}

fn gate_projection_prompt_context(
    blocked_prompt: Option<&ProductOutboundPayload>,
) -> Result<GateProjectionPromptContext, ProductAdapterError> {
    let context = match blocked_prompt {
        Some(ProductOutboundPayload::GatePrompt(prompt)) => GateProjectionPromptContext {
            invocation_id: prompt.invocation_id,
            headline: Some(prompt.headline.clone()),
            body: Some(prompt.body.clone()),
            allow_always: Some(prompt.allow_always),
            auth_context: None,
        },
        // The channel-connection context (for a manual_token pairing gate)
        // rides inside `auth_context` (`AuthPromptContextView::connection`) — the
        // single canonical place on the gate item, no duplicate top-level field.
        Some(ProductOutboundPayload::AuthPrompt(prompt)) => GateProjectionPromptContext {
            invocation_id: prompt.invocation_id,
            headline: Some(prompt.headline.clone()),
            body: Some(prompt.body.clone()),
            allow_always: Some(false),
            auth_context: AuthPromptContextView::from_auth_prompt(prompt)?,
        },
        _ => GateProjectionPromptContext::default(),
    };
    Ok(context)
}

fn gate_projection_item(
    event: &TurnLifecycleEvent,
    blocked_prompt: Option<&ProductOutboundPayload>,
) -> Result<Option<ProductProjectionItem>, ProductAdapterError> {
    if !matches!(event.kind, TurnEventKind::Blocked) {
        return Ok(None);
    }
    let Some(blocked_gate) = event.blocked_gate.as_ref() else {
        return Ok(None);
    };
    let prompt_context = gate_projection_prompt_context(blocked_prompt)?;
    let blocked_invocation_id = blocked_gate
        .activity_id
        .map(|activity_id| InvocationId::from_uuid(activity_id.as_uuid()));
    let body = prompt_context.body.unwrap_or_else(|| {
        event
            .sanitized_reason
            .clone()
            .unwrap_or_else(|| gate_projection_body(blocked_gate.gate_kind).to_string())
    });
    Ok(Some(ProductProjectionItem::Gate {
        run_id: event.run_id,
        gate_kind: product_gate_kind(blocked_gate.gate_kind),
        gate_ref: blocked_gate.gate_ref.as_str().to_string(),
        invocation_id: prompt_context.invocation_id.or(blocked_invocation_id),
        headline: prompt_context
            .headline
            .unwrap_or_else(|| gate_projection_headline(blocked_gate.gate_kind).to_string()),
        body: Some(body),
        allow_always: prompt_context.allow_always.unwrap_or(false),
        auth_context: prompt_context.auth_context.map(Box::new),
    }))
}

fn product_gate_kind(kind: TurnBlockedGateKind) -> ProductGateKind {
    match kind {
        TurnBlockedGateKind::Approval => ProductGateKind::Approval,
        TurnBlockedGateKind::Auth => ProductGateKind::Auth,
        TurnBlockedGateKind::Resource => ProductGateKind::Resource,
        TurnBlockedGateKind::AwaitDependentRun => ProductGateKind::Generic,
        TurnBlockedGateKind::ExternalTool => ProductGateKind::Generic,
    }
}

fn gate_projection_headline(kind: TurnBlockedGateKind) -> &'static str {
    match kind {
        TurnBlockedGateKind::Approval => "Approval required",
        TurnBlockedGateKind::Auth => "Authentication required",
        TurnBlockedGateKind::Resource => "Resource unavailable",
        TurnBlockedGateKind::AwaitDependentRun => "Waiting for dependent run",
        TurnBlockedGateKind::ExternalTool => "External tool call pending",
    }
}

fn gate_projection_body(kind: TurnBlockedGateKind) -> &'static str {
    match kind {
        TurnBlockedGateKind::Approval => "Resolve this approval gate to continue the run.",
        TurnBlockedGateKind::Auth => "Authenticate to continue this run.",
        TurnBlockedGateKind::Resource => "Resolve this resource gate to continue the run.",
        TurnBlockedGateKind::AwaitDependentRun => "Waiting for a dependent run to finish.",
        TurnBlockedGateKind::ExternalTool => "Submit the external tool output to continue the run.",
    }
}

#[derive(Debug, Clone, Default)]
struct FailureProjectionDetails {
    category: Option<SanitizedFailure>,
    summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct FailureExplanationCacheKey {
    run_id: TurnRunId,
    category: String,
}

#[derive(Debug)]
pub(super) struct FailureExplanationCache {
    capacity: usize,
    entries: HashMap<FailureExplanationCacheKey, Arc<OnceCell<String>>>,
    order: VecDeque<FailureExplanationCacheKey>,
}

impl FailureExplanationCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn cell_for(&mut self, key: FailureExplanationCacheKey) -> Arc<OnceCell<String>> {
        if let Some(cell) = self.entries.get(&key) {
            return Arc::clone(cell);
        }
        if self.entries.len() >= self.capacity
            && let Some(evicted) = self.order.pop_front()
        {
            self.entries.remove(&evicted);
        }
        let cell = Arc::new(OnceCell::new());
        self.entries.insert(key.clone(), Arc::clone(&cell));
        self.order.push_back(key);
        cell
    }
}

async fn failure_details_for_turn_event(
    failure_explainer: &dyn FailureExplanationProvider,
    failure_explanation_cache: &Arc<Mutex<FailureExplanationCache>>,
    event: &TurnLifecycleEvent,
) -> FailureProjectionDetails {
    let Some(category) = failure_category_for_turn_event(event) else {
        return FailureProjectionDetails::default();
    };
    // The model-visible raw cause for the failure travels on the failure
    // record's detail channel, surfaced on `TurnLifecycleEvent.detail` by the
    // upstream runner. Source it here so the explainer (and model) get the real
    // cause instead of only the bounded category.
    let detail = detail_for_turn_event(event, &category);
    let invalid_output_detail =
        ModelInvalidOutputDetailReason::from_failure_category_and_safe_summary(
            &category,
            detail.as_deref(),
        );
    let fallback_summary =
        reborn_failure_summary_for_category_and_detail(Some(&category), invalid_output_detail)
            .to_string();
    let cache_key = FailureExplanationCacheKey {
        run_id: event.run_id,
        category: category.clone(),
    };
    let summary = cached_failure_summary(failure_explanation_cache, cache_key, || async {
        failure_summary_for_turn_event(failure_explainer, &category, fallback_summary, detail).await
    })
    .await;
    FailureProjectionDetails {
        category: SanitizedFailure::new(category).ok(),
        summary: Some(summary),
    }
}

async fn cached_failure_summary<F, Fut>(
    cache: &Arc<Mutex<FailureExplanationCache>>,
    key: FailureExplanationCacheKey,
    compute: F,
) -> String
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = String>,
{
    let cell = cache.lock().await.cell_for(key);
    cell.get_or_init(compute).await.clone()
}

async fn failure_summary_for_turn_event(
    failure_explainer: &dyn FailureExplanationProvider,
    category: &str,
    fallback_summary: String,
    detail: Option<String>,
) -> String {
    if category == CHECKPOINT_REJECTED_CATEGORY {
        return checkpoint_rejection_host_explanation_from_detail(detail.as_deref())
            .unwrap_or(fallback_summary);
    }
    if let Some(summary) = pinned_failure_summary_for_category(category) {
        return summary.to_string();
    }
    failure_explainer
        .explain_failure(FailureExplanationInput {
            failure_category: category.to_string(),
            fallback_summary: fallback_summary.clone(),
            detail,
        })
        .await
        .unwrap_or(fallback_summary)
}

/// Resolve the model-visible detail to hand the failure explainer.
///
/// Sources the secret-scrubbed raw cause from `TurnLifecycleEvent.detail`, the
/// dedicated channel distinct from `sanitized_reason` (which is consumed as the
/// `failure_category`). The detail originates on
/// `AgentLoopHostError`/`HostManagedModelError` upstream and is threaded through
/// the executor/driver diagnostics into the failure record and onto the event.
/// `None` when the failed run recorded no distinct cause, in which case the
/// explainer falls back to the category summary.
fn detail_for_turn_event(event: &TurnLifecycleEvent, _category: &str) -> Option<String> {
    event.detail.clone()
}

fn failure_category_for_turn_event(event: &TurnLifecycleEvent) -> Option<String> {
    matches!(
        event.status,
        TurnStatus::Failed | TurnStatus::RecoveryRequired
    )
    .then(|| event.sanitized_reason.clone())
    .flatten()
}

fn failure_explanation_request(input: &FailureExplanationInput) -> Option<SystemInferenceRequest> {
    Some(SystemInferenceRequest {
        task_id: SystemInferenceTaskId::new(),
        identity: SystemInferenceIdentity {
            task_kind: SystemTaskKind::FailureExplanation,
            prompt_source: SystemPromptSource::Static {
                prompt_id: SystemPromptId::new("failure_explanation").ok()?,
            },
            system_prompt: failure_explanation_system_prompt().to_string(),
        },
        input_text: failure_explanation_user_prompt(input),
        max_input_tokens: FAILURE_EXPLANATION_MAX_INPUT_TOKENS,
        deadline_ms: FAILURE_EXPLANATION_TIMEOUT
            .as_millis()
            .min(u128::from(u64::MAX)) as u64,
    })
}

/// The failure-explainer system prompt.
///
/// The asset is product-owned (WS1.7): this projection is its only consumer,
/// and prompt *content* is explicitly out of charter for the loop tier
/// (PROPOSAL §6.1.4/§6.7.2), where it used to live as
/// `ironclaw_loop_host::FAILURE_EXPLANATION_SYSTEM_PROMPT`.
fn failure_explanation_system_prompt() -> &'static str {
    FAILURE_EXPLANATION_SYSTEM_PROMPT
}

/// Product-owned prompt asset; see [`failure_explanation_system_prompt`].
const FAILURE_EXPLANATION_SYSTEM_PROMPT: &str =
    include_str!("../../prompts/failure_explanation.md");

pub(super) fn failure_explanation_user_prompt(input: &FailureExplanationInput) -> String {
    let mut prompt = format!(
        "status: failed\nfailure_category: {}\nfallback_summary: {}\n",
        sanitize_model_visible_text(&input.failure_category),
        sanitize_model_visible_text(&input.fallback_summary),
    );
    // Give the explainer the real cause when one survived from the failure
    // record. Re-run the value-scrubber as defense in depth even though the
    // producer is expected to have scrubbed secret VALUES already; skip the
    // line entirely when the detail is absent or scrubs to empty.
    //
    // Unlike `failure_category`/`fallback_summary` (host-authored, derived from
    // the category), `detail` is untrusted provider/tool/runtime error text.
    // `sanitize_model_visible_text` redacts credential-shaped tokens but keeps
    // newlines and instruction-like content, so appending it raw would let a
    // crafted error (e.g. from an MCP server or provider body) inject extra
    // prompt fields or directives — a `\nfallback_summary: ...\nIgnore previous
    // instructions...` — into the explainer, whose output becomes the public
    // `failure_summary`. JSON-string-escape it so it stays a single quoted data
    // value the model reads as data, never as prompt structure.
    if let Some(detail) = input.detail.as_deref() {
        let scrubbed = sanitize_model_visible_text(detail);
        if !scrubbed.trim().is_empty() {
            let quoted = serde_json::to_string(&scrubbed)
                .unwrap_or_else(|_| "\"<undisplayable detail>\"".to_string());
            prompt.push_str(&format!("detail: {quoted}\n"));
        }
    }
    prompt
}

pub(super) fn bounded_failure_explanation(content: &str) -> Option<String> {
    let sanitized = sanitize_model_visible_text(content).trim().to_string();
    if sanitized.is_empty() {
        return None;
    }
    if sanitized.len() <= FAILURE_EXPLANATION_MAX_BYTES {
        return Some(sanitized);
    }
    let mut end = FAILURE_EXPLANATION_MAX_BYTES;
    while end > 0 && !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = sanitized[..end].trim_end().to_string();
    (!truncated.is_empty()).then_some(truncated)
}

pub(super) fn turn_status_wire(status: TurnStatus) -> &'static str {
    match status {
        TurnStatus::Queued => "queued",
        TurnStatus::Running => "running",
        TurnStatus::BlockedApproval => "blocked_approval",
        TurnStatus::BlockedAuth => "blocked_auth",
        TurnStatus::BlockedResource => "blocked_resource",
        TurnStatus::BlockedDependentRun => "blocked_dependent_run",
        TurnStatus::BlockedExternalTool => "blocked_external_tool",
        TurnStatus::RecoveryRequired => "recovery_required",
        TurnStatus::CancelRequested => "cancel_requested",
        TurnStatus::Completed => "completed",
        TurnStatus::Cancelled => "cancelled",
        TurnStatus::Failed => "failed",
    }
}

fn map_turn_event_projection_error(error: TurnEventProjectionError) -> ProductAdapterError {
    tracing::warn!(
        component = "turn_event_projection",
        operation = "map_turn_event_projection_error",
        error = %error,
        error_debug = ?error,
        "turn event projection error mapped to product adapter error"
    );
    match error {
        TurnEventProjectionError::InvalidRequest { reason } => {
            ProductAdapterError::InvalidIdentifier {
                kind: "projection_cursor",
                reason: reason.to_string(),
            }
        }
        TurnEventProjectionError::RebaseRequired {
            requested,
            earliest,
        } if requested.scope != earliest.scope => ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            reason: "turn cursor scope does not match subscription scope".to_string(),
        },
        TurnEventProjectionError::RebaseRequired { .. } => ProductAdapterError::SurfaceRejected {
            kind: ProductSurfaceRejectionKind::Unavailable,
            status_code: 503,
            retryable: true,
            reason: RedactedString::new("turn event projection rebase required; reconnect"),
        },
        TurnEventProjectionError::Source { .. } => ProductAdapterError::SurfaceRejected {
            kind: ProductSurfaceRejectionKind::Unavailable,
            status_code: 503,
            retryable: true,
            reason: RedactedString::new("turn event projection source unavailable"),
        },
    }
}
