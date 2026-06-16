use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use async_trait::async_trait;
use futures::{StreamExt, stream};
use ironclaw_host_api::{Action, ApprovalRequest, NetworkMethod, NetworkScheme, UserId};
use ironclaw_product_adapters::{
    ApprovalPromptActionView, ApprovalPromptContextView, ApprovalPromptDestinationView,
    ApprovalPromptDetailView, ApprovalPromptScopeView, GatePromptView, ProductAdapterError,
    ProductOutboundPayload, ProductProjectionItem, ProductProjectionState,
    ProductWorkflowRejectionKind, RedactedString,
};
use ironclaw_product_workflow::{
    ApprovalInteractionScope, approval_request_id_from_gate_ref, is_approval_gate_ref,
};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_turns::{
    GateRef, GetRunStateRequest, SanitizedFailure, TurnActor, TurnCoordinator, TurnError,
    TurnEventKind, TurnEventProjectionCursor, TurnEventProjectionError, TurnEventProjectionRequest,
    TurnEventProjectionService, TurnEventProjectionSource, TurnLifecycleEvent, TurnRunId,
    TurnScope, TurnStatus,
    run_profile::{
        SystemInferenceIdentity, SystemInferencePort, SystemInferenceRequest,
        SystemInferenceTaskId, SystemPromptId, SystemPromptSource, SystemTaskKind,
        sanitize_model_visible_text,
    },
};
use tokio::sync::{Mutex, OnceCell, Semaphore};

use crate::AuthChallengeProvider;
use crate::auth_prompt::auth_prompt_view_for_blocked_auth;
use crate::failure_summary::{
    pinned_failure_summary_for_category, reborn_failure_summary_for_category,
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
pub(crate) struct FailureExplanationInput {
    pub(crate) failure_category: String,
    pub(crate) fallback_summary: String,
}

#[async_trait]
pub(crate) trait FailureExplanationProvider: Send + Sync {
    async fn explain_failure(&self, input: FailureExplanationInput) -> Option<String>;
}

#[derive(Debug, Default)]
pub(crate) struct NoopFailureExplanationProvider;

pub(super) struct TurnEventDrain {
    pub(super) next_cursor: Option<TurnEventProjectionCursor>,
    pub(super) payloads: Vec<TurnEventPayload>,
}

#[derive(Clone, Default)]
pub(super) enum TurnEventBridge {
    #[default]
    Disabled,
    Enabled {
        service: Arc<TurnEventProjectionService<dyn TurnEventProjectionSource>>,
        coordinator: Arc<dyn TurnCoordinator>,
        approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
        failure_explainer: Arc<dyn FailureExplanationProvider>,
        failure_explanation_cache: Arc<Mutex<FailureExplanationCache>>,
    },
}

pub(crate) struct ModelFailureExplanationProvider {
    system_inference: Arc<dyn Fn() -> Arc<dyn SystemInferencePort> + Send + Sync>,
    permits: Arc<Semaphore>,
}

impl ModelFailureExplanationProvider {
    #[cfg(test)]
    pub(crate) fn new(system_inference: Arc<dyn SystemInferencePort>) -> Self {
        Self {
            system_inference: Arc::new(move || Arc::clone(&system_inference)),
            permits: Arc::new(Semaphore::new(
                FAILURE_EXPLANATION_MAX_CONCURRENT_MODEL_CALLS,
            )),
        }
    }

    pub(crate) fn from_factory(
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
        approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
    ) -> Self {
        Self::Enabled {
            service: Arc::new(TurnEventProjectionService::new(source)),
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
        requests: Option<Arc<dyn ApprovalRequestStore>>,
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
        caller_user_id: &ironclaw_host_api::UserId,
        scope: &TurnScope,
        after: Option<TurnEventProjectionCursor>,
        auth_challenges: Option<&dyn AuthChallengeProvider>,
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
                Err(TurnEventProjectionError::RebaseRequired { earliest, .. })
                    if after_cursor.is_none() =>
                {
                    return Ok(TurnEventDrain {
                        next_cursor: Some(*earliest),
                        payloads: Vec::new(),
                    });
                }
                Err(error) => return Err(map_turn_event_projection_error(error)),
            };
            next_cursor = Some(page.next_cursor.clone());
            payloads.extend(
                turn_event_payloads_for_page(
                    caller_user_id,
                    coordinator.as_ref(),
                    failure_explainer.as_ref(),
                    failure_explanation_cache,
                    auth_challenges,
                    approval_requests.as_deref(),
                    page.entries,
                )
                .await?,
            );
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

async fn turn_event_payloads_for_page(
    caller_user_id: &ironclaw_host_api::UserId,
    coordinator: &dyn TurnCoordinator,
    failure_explainer: &dyn FailureExplanationProvider,
    failure_explanation_cache: &Arc<Mutex<FailureExplanationCache>>,
    auth_challenges: Option<&dyn AuthChallengeProvider>,
    approval_requests: Option<&dyn ApprovalRequestStore>,
    events: Vec<TurnLifecycleEvent>,
) -> Result<Vec<TurnEventPayload>, ProductAdapterError> {
    let futures = events.into_iter().map(|event| {
        let cursor = TurnEventProjectionCursor::for_scope(event.scope.clone(), event.cursor);
        async move {
            turn_event_payload(
                caller_user_id,
                coordinator,
                failure_explainer,
                failure_explanation_cache,
                auth_challenges,
                approval_requests,
                &event,
            )
            .await
            .map(|payload| payload.map(|payload| TurnEventPayload { cursor, payload }))
        }
    });
    stream::iter(futures)
        .buffered(16)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter_map(Result::transpose)
        .collect()
}

async fn turn_event_payload(
    caller_user_id: &ironclaw_host_api::UserId,
    coordinator: &dyn TurnCoordinator,
    failure_explainer: &dyn FailureExplanationProvider,
    failure_explanation_cache: &Arc<Mutex<FailureExplanationCache>>,
    auth_challenges: Option<&dyn AuthChallengeProvider>,
    approval_requests: Option<&dyn ApprovalRequestStore>,
    event: &TurnLifecycleEvent,
) -> Result<Option<ProductOutboundPayload>, ProductAdapterError> {
    if matches!(event.kind, TurnEventKind::Blocked)
        && let Some(prompt) = blocked_prompt_payload(
            caller_user_id,
            coordinator,
            auth_challenges,
            approval_requests,
            event,
        )
        .await?
    {
        return Ok(Some(prompt));
    }
    if projects_run_status(&event.kind) {
        let failure_details =
            failure_details_for_turn_event(failure_explainer, failure_explanation_cache, event)
                .await;
        return Ok(Some(ProductOutboundPayload::ProjectionUpdate {
            state: turn_event_projection_state(event, failure_details)?,
        }));
    }
    Ok(None)
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
    caller_user_id: &ironclaw_host_api::UserId,
    coordinator: &dyn TurnCoordinator,
    auth_challenges: Option<&dyn AuthChallengeProvider>,
    approval_requests: Option<&dyn ApprovalRequestStore>,
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
            return Err(ProductAdapterError::WorkflowTransient {
                reason: RedactedString::new("turn gate state lookup failed"),
            });
        }
    };
    if state.status != event.status || state.event_cursor != event.cursor {
        return Ok(None);
    }
    let Some(gate_ref) = state.gate_ref.as_ref() else {
        return Ok(None);
    };
    let gate_ref_str = gate_ref.as_str().to_string();
    match event.status {
        TurnStatus::BlockedAuth => {
            let view = auth_prompt_view_for_blocked_auth(
                event.owner_user_id.as_ref().unwrap_or(caller_user_id),
                &event.scope,
                event.run_id,
                &gate_ref_str,
                event
                    .sanitized_reason
                    .clone()
                    .unwrap_or_else(|| "Authenticate to continue this run.".to_string()),
                &state.credential_requirements,
                auth_challenges,
            )
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
            .await,
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
        | TurnStatus::RecoveryRequired
        | TurnStatus::CancelRequested
        | TurnStatus::Completed
        | TurnStatus::Cancelled
        | TurnStatus::Failed => Ok(None),
    }
}

async fn approval_gate_prompt(
    caller_user_id: &UserId,
    approval_requests: Option<&dyn ApprovalRequestStore>,
    event: &TurnLifecycleEvent,
    gate_ref: &GateRef,
    gate_ref_string: String,
) -> ProductOutboundPayload {
    let owner_user_id = event.owner_user_id.as_ref().unwrap_or(caller_user_id);
    let context =
        approval_prompt_context_view(approval_requests, gate_ref, owner_user_id, &event.scope)
            .await;
    gate_prompt_with_context(
        event,
        gate_ref_string,
        "Approval required",
        is_approval_gate_ref(gate_ref.as_str()),
        context,
    )
}

/// Resolve an approval gate's request details (tool/action/reason) into the
/// rendered context view, by looking it up in the `ApprovalRequestStore` by
/// gate ref. Shared by the WebUI gate projection and the Slack approval prompt
/// so both surface the *same* "what is being approved" data from one source.
/// Returns `None` when no store is wired, the gate ref is not an approval ref,
/// the request is missing, or the lookup fails.
pub(crate) async fn approval_prompt_context_view(
    approval_requests: Option<&dyn ApprovalRequestStore>,
    gate_ref: &GateRef,
    owner_user_id: &UserId,
    turn_scope: &TurnScope,
) -> Option<ApprovalPromptContextView> {
    let (store, request_id) =
        approval_requests.zip(approval_request_id_from_gate_ref(gate_ref).ok())?;
    let scope =
        ApprovalInteractionScope::from_turn(turn_scope, &TurnActor::new(owner_user_id.clone()))
            .to_resource_scope();
    match store.get(&scope, request_id).await {
        Ok(Some(record)) => approval_context_for_request(&record.request),
        Ok(None) => None,
        Err(error) => {
            tracing::debug!(
                %error,
                request_id = %request_id,
                "approval request lookup failed during gate projection"
            );
            // silent-ok: approval context is best-effort UI enrichment; gate prompts remain actionable without it
            None
        }
    }
}

fn approval_context_for_request(request: &ApprovalRequest) -> Option<ApprovalPromptContextView> {
    let (tool_name, action, destination, details) =
        approval_action_context(request.action.as_ref())?;
    ApprovalPromptContextView::new(
        tool_name,
        action,
        ApprovalPromptScopeView::new(
            approval_scope_label(request),
            request.reusable_scope.is_some(),
        )
        .ok()?,
        non_empty_string(&request.reason),
        destination,
        details,
    )
    .ok()
}

fn approval_action_context(
    action: &Action,
) -> Option<(
    String,
    ApprovalPromptActionView,
    Option<ApprovalPromptDestinationView>,
    Vec<ApprovalPromptDetailView>,
)> {
    match action {
        Action::Dispatch {
            capability,
            estimated_resources,
        } => {
            let mut details = vec![detail("Capability", capability.as_str())?];
            if let Some(bytes) = estimated_resources.network_egress_bytes {
                details.push(detail("Estimated network egress", format_bytes(bytes))?);
            }
            Some((
                capability.as_str().to_string(),
                ApprovalPromptActionView::new("Run tool", None).ok()?,
                None,
                details,
            ))
        }
        Action::SpawnCapability {
            capability,
            estimated_resources,
        } => {
            let mut details = vec![detail("Capability", capability.as_str())?];
            if let Some(process_count) = estimated_resources.process_count {
                details.push(detail("Processes", process_count.to_string())?);
            }
            Some((
                capability.as_str().to_string(),
                ApprovalPromptActionView::new("Start tool", None).ok()?,
                None,
                details,
            ))
        }
        Action::Network {
            target,
            method,
            estimated_bytes,
        } => {
            let destination =
                network_destination(method, target.scheme, &target.host, target.port)?;
            let mut details = vec![detail("Method", method_label(method))?];
            if let Some(bytes) = estimated_bytes {
                details.push(detail("Estimated transfer", format_bytes(*bytes))?);
            }
            Some((
                "builtin.http".to_string(),
                ApprovalPromptActionView::new("Network request", Some(*method)).ok()?,
                Some(destination),
                details,
            ))
        }
        _ => None,
    }
}

fn approval_scope_label(request: &ApprovalRequest) -> &'static str {
    if request.reusable_scope.is_some() {
        "Reusable grant"
    } else {
        "This request only"
    }
}

fn network_destination(
    method: &NetworkMethod,
    scheme: NetworkScheme,
    host: &str,
    port: Option<u16>,
) -> Option<ApprovalPromptDestinationView> {
    let scheme = match scheme {
        NetworkScheme::Http => "http",
        NetworkScheme::Https => "https",
    };
    let authority = match port {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    let url = format!("{scheme}://{authority}");
    ApprovalPromptDestinationView::new(
        format!("{} {url}", method_label(method)),
        Some(url),
        Some(host.to_string()),
    )
    .ok()
}

fn detail(label: impl Into<String>, value: impl Into<String>) -> Option<ApprovalPromptDetailView> {
    ApprovalPromptDetailView::new(label, value).ok()
}

fn method_label(method: &NetworkMethod) -> String {
    method.to_string().to_ascii_uppercase()
}

fn format_bytes(bytes: u64) -> String {
    format!("{bytes} bytes")
}

fn non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn gate_prompt(
    event: &TurnLifecycleEvent,
    gate_ref: String,
    headline: &'static str,
    allow_always: bool,
) -> ProductOutboundPayload {
    gate_prompt_with_context(event, gate_ref, headline, allow_always, None)
}

fn gate_prompt_with_context(
    event: &TurnLifecycleEvent,
    gate_ref: String,
    headline: &'static str,
    allow_always: bool,
    approval_context: Option<ApprovalPromptContextView>,
) -> ProductOutboundPayload {
    ProductOutboundPayload::GatePrompt(GatePromptView {
        turn_run_id: event.run_id,
        gate_ref,
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
) -> Result<ProductProjectionState, ProductAdapterError> {
    ProductProjectionState::new(
        event.scope.thread_id.to_string(),
        vec![ProductProjectionItem::RunStatus {
            run_id: event.run_id,
            status: turn_status_wire(event.status).to_string(),
            failure_category: failure_details.category,
            failure_summary: failure_details.summary,
        }],
    )
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
    let fallback_summary = reborn_failure_summary_for_category(Some(&category)).to_string();
    let cache_key = FailureExplanationCacheKey {
        run_id: event.run_id,
        category: category.clone(),
    };
    let summary = cached_failure_summary(failure_explanation_cache, cache_key, || async {
        failure_summary_for_turn_event(failure_explainer, &category, fallback_summary).await
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
) -> String {
    if let Some(summary) = pinned_failure_summary_for_category(category) {
        return summary.to_string();
    }
    failure_explainer
        .explain_failure(FailureExplanationInput {
            failure_category: category.to_string(),
            fallback_summary: fallback_summary.clone(),
        })
        .await
        .unwrap_or(fallback_summary)
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

fn failure_explanation_system_prompt() -> &'static str {
    ironclaw_loop_support::FAILURE_EXPLANATION_SYSTEM_PROMPT
}

fn failure_explanation_user_prompt(input: &FailureExplanationInput) -> String {
    format!(
        "status: failed\nfailure_category: {}\nfallback_summary: {}\n",
        sanitize_model_visible_text(&input.failure_category),
        sanitize_model_visible_text(&input.fallback_summary),
    )
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

fn turn_status_wire(status: TurnStatus) -> &'static str {
    match status {
        TurnStatus::Queued => "queued",
        TurnStatus::Running => "running",
        TurnStatus::BlockedApproval => "blocked_approval",
        TurnStatus::BlockedAuth => "blocked_auth",
        TurnStatus::BlockedResource => "blocked_resource",
        TurnStatus::BlockedDependentRun => "blocked_dependent_run",
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
        TurnEventProjectionError::RebaseRequired { .. } => ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unavailable,
            status_code: 503,
            retryable: true,
            reason: RedactedString::new("turn event projection rebase required; reconnect"),
        },
        TurnEventProjectionError::Source { .. } => ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unavailable,
            status_code: 503,
            retryable: true,
            reason: RedactedString::new("turn event projection source unavailable"),
        },
    }
}
