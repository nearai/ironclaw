//! WebUI service facade for native Reborn WebChat v2 (issue #3611).
//!
//! Browser-facing route handlers depend only on this facade. They must not
//! reach the dispatcher, run-state store, runtime-lane adapters, or raw turn
//! coordinator — those live behind [`WebUiService`].
//!
//! This is the Path A (native host surface) seam described in
//! `docs/reborn/how-to-port-channel-to-reborn.md`. WebUI sessions are
//! host-trusted, so this facade does **not** fabricate `ExternalActorRef`,
//! `ProtocolAuthEvidence`, declared egress, or `OutboundDeliverySink`.

use async_trait::async_trait;
use ironclaw_event_projections::{
    EventProjectionService, MAX_PROJECTION_PAGE_LIMIT, ProjectionCursor, ProjectionError,
    ProjectionRequest, ProjectionScope, RunStatusProjection, TimelineEntry,
};
use ironclaw_events::{EventStreamKey, ReadScope};
use ironclaw_host_api::ThreadId;
use ironclaw_threads::{
    EnsureThreadRequest, MessageContent, SessionThreadError, SessionThreadService, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, GateRef, IdempotencyKey, ReplyTargetBindingRef,
    ResumeTurnRequest, SanitizedCancelReason, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnErrorCategory, TurnRunId,
    TurnScope,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::webui_inbound::{WebUiAuthenticatedCaller, WebUiGateResolution};

/// Default page size if the caller asks for `0` or omits the limit.
pub const WEBUI_TIMELINE_DEFAULT_LIMIT: usize = 100;

// ---------------------------------------------------------------------------
// Public facade trait
// ---------------------------------------------------------------------------

/// Browser-facing WebUI command surface.
///
/// Route handlers consume only this trait. Implementations route each command
/// to the appropriate Reborn host service (thread service, turn coordinator,
/// future gate-resolve port) without exposing those services to handlers.
#[async_trait]
pub trait WebUiService: Send + Sync {
    /// Create or ensure a thread for the authenticated caller.
    async fn create_thread(
        &self,
        command: WebUiCreateThreadCommand,
    ) -> Result<WebUiThreadCreated, WebUiServiceError>;

    /// Accept a user message and submit a turn (or defer it if the thread is busy).
    async fn send_message(
        &self,
        command: WebUiSendMessageCommand,
    ) -> Result<WebUiMessageAccepted, WebUiServiceError>;

    /// Request cancellation of an in-flight run.
    async fn cancel_run(
        &self,
        command: WebUiCancelRunCommand,
    ) -> Result<WebUiRunCancelled, WebUiServiceError>;

    /// Resolve an approval/auth/resource gate that an active run is parked on.
    async fn resolve_gate(
        &self,
        command: WebUiResolveGateCommand,
    ) -> Result<WebUiGateResolved, WebUiServiceError>;

    /// Initial timeline snapshot for a thread (used to bootstrap the chat view).
    async fn get_timeline_snapshot(
        &self,
        command: WebUiGetTimelineCommand,
    ) -> Result<WebUiTimelineSnapshot, WebUiServiceError>;

    /// Single batch of timeline entries that arrived after the supplied cursor.
    ///
    /// The browser-facing SSE handler builds the actual server-sent-events
    /// loop on top of this method; this trait only exposes one batch read so
    /// the facade stays transport-agnostic.
    async fn get_timeline_updates(
        &self,
        command: WebUiGetTimelineCommand,
    ) -> Result<WebUiTimelineReplay, WebUiServiceError>;
}

// ---------------------------------------------------------------------------
// Per-command input structs
// ---------------------------------------------------------------------------

/// Input for [`WebUiService::create_thread`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUiCreateThreadCommand {
    pub caller: WebUiAuthenticatedCaller,
    pub client_action_id: IdempotencyKey,
    pub requested_thread_id: Option<ThreadId>,
}

/// Input for [`WebUiService::send_message`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUiSendMessageCommand {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub client_action_id: IdempotencyKey,
    pub content: String,
}

/// Input for [`WebUiService::cancel_run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUiCancelRunCommand {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub run_id: TurnRunId,
    pub reason: SanitizedCancelReason,
    pub client_action_id: IdempotencyKey,
}

/// Input for [`WebUiService::resolve_gate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUiResolveGateCommand {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub run_id: TurnRunId,
    pub gate_ref: GateRef,
    pub client_action_id: IdempotencyKey,
    pub resolution: WebUiGateResolution,
}

/// Input for the timeline read methods.
///
/// `after` is opaque to handlers — pass the [`WebUiTimelineSnapshot::next_cursor`]
/// or [`WebUiTimelineReplay::next_cursor`] from the previous batch unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUiGetTimelineCommand {
    pub caller: WebUiAuthenticatedCaller,
    pub thread_id: ThreadId,
    pub after: Option<WebUiTimelineCursor>,
    pub limit: usize,
}

/// Opaque cursor that the browser passes back into subsequent timeline reads.
///
/// Handlers may serialize the wrapped JSON to the browser but must not
/// reach into the inner projection cursor. `#[serde(transparent)]` keeps
/// the wire shape identical to the inner `ProjectionCursor`, so browsers
/// see a clean JSON object instead of the default tuple-struct encoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WebUiTimelineCursor(ProjectionCursor);

impl WebUiTimelineCursor {
    pub(crate) fn from_projection(cursor: ProjectionCursor) -> Self {
        Self(cursor)
    }

    pub(crate) fn into_projection(self) -> ProjectionCursor {
        self.0
    }

    pub(crate) fn as_projection(&self) -> &ProjectionCursor {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// Per-command outcome types
// ---------------------------------------------------------------------------

/// Successful outcome of [`WebUiService::create_thread`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUiThreadCreated {
    pub thread_id: ThreadId,
}

/// Successful outcome of [`WebUiService::send_message`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUiMessageAccepted {
    pub thread_id: ThreadId,
    pub accepted_message_ref: AcceptedMessageRef,
    pub run: WebUiMessageRunOutcome,
}

/// Whether the submitted message produced a new run or was deferred behind an
/// active run on the same thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebUiMessageRunOutcome {
    /// A new run was admitted by the turn coordinator.
    Submitted { run_id: TurnRunId },
    /// The thread already had an active run; this message is queued behind it.
    DeferredBusy { active_run_id: TurnRunId },
}

/// Successful outcome of [`WebUiService::cancel_run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUiRunCancelled {
    pub run_id: TurnRunId,
    pub already_terminal: bool,
}

/// Successful outcome of [`WebUiService::resolve_gate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebUiGateResolved {
    /// Gate approved (or a credential was supplied) — run resumed.
    Resumed { run_id: TurnRunId },
    /// Gate denied or cancelled by the user — run cancellation requested.
    Cancelled {
        run_id: TurnRunId,
        already_terminal: bool,
    },
}

/// Initial snapshot result returned by [`WebUiService::get_timeline_snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUiTimelineSnapshot {
    pub entries: Vec<TimelineEntry>,
    pub runs: Vec<RunStatusProjection>,
    pub next_cursor: WebUiTimelineCursor,
    pub truncated: bool,
}

/// Update batch result returned by [`WebUiService::get_timeline_updates`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUiTimelineReplay {
    pub entries: Vec<TimelineEntry>,
    pub runs: Vec<RunStatusProjection>,
    pub next_cursor: WebUiTimelineCursor,
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// Error vocabulary
// ---------------------------------------------------------------------------

/// Redacted error surface for WebUI handlers.
///
/// All internal reasons (provider details, host paths, raw store errors) are
/// summarized into stable variants so handlers can map them to HTTP status
/// codes via [`WebUiServiceError::status_code`] without leaking
/// provider/internal detail.
///
/// Variants are deliberately classified by **what the browser should do**
/// (re-snapshot, retry, prompt, give up) rather than by which downstream
/// service produced the error, so a single redaction rule applies regardless
/// of source.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WebUiServiceError {
    /// Caller lacks an agent binding required for the requested operation.
    #[error("caller is missing required agent context")]
    MissingAgentContext,

    /// The requested resource (thread, run, message) does not exist for
    /// this caller.
    #[error("resource not found")]
    NotFound,

    /// The caller is not authorized for this resource. The most common case
    /// is a thread that exists under a different `(tenant, agent)` scope.
    #[error("forbidden")]
    Forbidden,

    /// The request conflicts with current state (e.g. transitioning a
    /// message from an incompatible status, idempotency key reused across
    /// different threads, message already past the draft phase).
    #[error("request conflicts with current state")]
    Conflict,

    /// Input failed shape validation inside the facade (e.g. cursor scope
    /// mismatch, invalid summary range, malformed ref).
    #[error("invalid input")]
    InvalidInput,

    /// The turn coordinator rejected the request with a typed category. The
    /// `status_code()` mapping is derived from the category so handlers
    /// don't need to know the turn-error vocabulary directly.
    #[error("turn coordinator rejected request")]
    TurnRejected { category: TurnErrorCategory },

    /// A transient downstream failure (durable store backend, serialization,
    /// projection source). Safe to retry.
    #[error("transient downstream failure")]
    Transient,

    /// The operation is recognized by the facade but the underlying capability
    /// is not yet wired in the current slice. Handlers should treat this as a
    /// permanent failure for the request, not a retryable one.
    ///
    /// Currently used for `WebUiGateResolution::CredentialProvided`, which
    /// requires a credential-binding port that does not exist in Slice 1.
    #[error("operation not yet supported: {what}")]
    Unsupported { what: &'static str },

    /// The supplied timeline cursor is older than the durable log can replay
    /// from. The browser must drop the cursor and call
    /// [`WebUiService::get_timeline_snapshot`] again to rebase. The opaque
    /// cursor returned here is the earliest available replay point.
    ///
    /// Boxed so the `Result` size on the happy path stays small — every
    /// facade method returns this error type.
    #[error("timeline cursor is too old; re-snapshot required")]
    TimelineRebaseRequired {
        earliest_cursor: Box<WebUiTimelineCursor>,
    },
}

impl WebUiServiceError {
    /// HTTP status code suggested for this error.
    pub fn status_code(&self) -> u16 {
        match self {
            Self::MissingAgentContext | Self::InvalidInput => 400,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            // 409 Conflict: thread/message state mismatch, or the browser's
            // view diverged from the durable log.
            Self::Conflict | Self::TimelineRebaseRequired { .. } => 409,
            Self::TurnRejected { category } => turn_category_status_code(*category),
            Self::Transient => 503,
            Self::Unsupported { .. } => 501,
        }
    }

    /// Whether this error is safe to retry from the browser.
    pub fn retryable(&self) -> bool {
        match self {
            Self::Transient => true,
            Self::TurnRejected { category } => {
                matches!(turn_category_status_code(*category), 429 | 503)
            }
            _ => false,
        }
    }
}

fn turn_category_status_code(category: TurnErrorCategory) -> u16 {
    match category {
        TurnErrorCategory::ThreadBusy | TurnErrorCategory::Conflict => 409,
        TurnErrorCategory::AdmissionRejected => 429,
        TurnErrorCategory::ScopeNotFound => 404,
        TurnErrorCategory::Unauthorized => 403,
        TurnErrorCategory::InvalidRequest => 400,
        TurnErrorCategory::Unavailable => 503,
    }
}

impl From<ProjectionError> for WebUiServiceError {
    fn from(value: ProjectionError) -> Self {
        match value {
            ProjectionError::InvalidRequest { .. } => Self::InvalidInput,
            ProjectionError::Source { .. } => Self::Transient,
            ProjectionError::RebaseRequired { earliest, .. } => Self::TimelineRebaseRequired {
                earliest_cursor: Box::new(WebUiTimelineCursor::from_projection(*earliest)),
            },
        }
    }
}

impl From<SessionThreadError> for WebUiServiceError {
    fn from(value: SessionThreadError) -> Self {
        match value {
            // Resource does not exist for this caller.
            SessionThreadError::UnknownThread { .. }
            | SessionThreadError::UnknownMessage { .. } => Self::NotFound,
            // Authorization boundary: the thread/idempotency key exists but
            // belongs to a different (tenant, agent) scope or different
            // canonical thread. Surface as 403 so the browser does not
            // infinitely retry against a forbidden resource.
            SessionThreadError::ThreadScopeMismatch { .. } => Self::Forbidden,
            // State precondition mismatch: the message already moved past the
            // status this operation needs, or the same idempotency key was
            // previously bound to a different thread.
            SessionThreadError::MessageNotDraft { .. }
            | SessionThreadError::InvalidMessageTransition { .. }
            | SessionThreadError::IdempotentReplayThreadMismatch { .. } => Self::Conflict,
            // Caller-supplied input is structurally invalid.
            SessionThreadError::InvalidSummaryRange { .. }
            | SessionThreadError::OverlappingSummaryRange { .. } => Self::InvalidInput,
            // Backend / generated-id / serialization failures are retryable.
            SessionThreadError::GeneratedThreadId(_)
            | SessionThreadError::Serialization(_)
            | SessionThreadError::Deserialization(_)
            | SessionThreadError::Backend(_) => Self::Transient,
        }
    }
}

impl From<TurnError> for WebUiServiceError {
    fn from(value: TurnError) -> Self {
        Self::TurnRejected {
            category: value.category(),
        }
    }
}

// ---------------------------------------------------------------------------
// Default implementation
// ---------------------------------------------------------------------------

/// Default `WebUiService` that composes a [`SessionThreadService`], a
/// [`TurnCoordinator`], and an [`EventProjectionService`].
pub struct DefaultWebUiService {
    thread_service: std::sync::Arc<dyn SessionThreadService>,
    turn_coordinator: std::sync::Arc<dyn TurnCoordinator>,
    projection_service: std::sync::Arc<dyn EventProjectionService>,
}

impl DefaultWebUiService {
    pub fn new(
        thread_service: std::sync::Arc<dyn SessionThreadService>,
        turn_coordinator: std::sync::Arc<dyn TurnCoordinator>,
        projection_service: std::sync::Arc<dyn EventProjectionService>,
    ) -> Self {
        Self {
            thread_service,
            turn_coordinator,
            projection_service,
        }
    }
}

#[async_trait]
impl WebUiService for DefaultWebUiService {
    async fn create_thread(
        &self,
        command: WebUiCreateThreadCommand,
    ) -> Result<WebUiThreadCreated, WebUiServiceError> {
        let WebUiCreateThreadCommand {
            caller,
            client_action_id,
            requested_thread_id,
        } = command;

        // C1 fix: when the browser omits `requested_thread_id`, derive a
        // deterministic id from the client-supplied idempotency key. A naive
        // `Uuid::new_v4()` here would create a fresh thread on every retry
        // (network drops, double-submit), violating the implicit idempotency
        // contract carried by `IdempotencyKey`.
        let thread_id = match requested_thread_id {
            Some(id) => id,
            None => derive_webui_thread_id(&client_action_id)?,
        };
        let scope = webui_thread_scope(&caller)?;

        let record = self
            .thread_service
            .ensure_thread(EnsureThreadRequest {
                scope,
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: caller.user_id.as_str().to_string(),
                title: None,
                metadata_json: None,
            })
            .await?;

        Ok(WebUiThreadCreated {
            thread_id: record.thread_id,
        })
    }

    async fn send_message(
        &self,
        command: WebUiSendMessageCommand,
    ) -> Result<WebUiMessageAccepted, WebUiServiceError> {
        let WebUiSendMessageCommand {
            scope,
            actor,
            client_action_id,
            content,
        } = command;

        let thread_scope = thread_scope_from_turn_scope(&scope, &actor)?;
        let thread_id = scope.thread_id.clone();

        // Idempotent thread ensure so the WebUI never wedges on a missing
        // session_threads row after a partial create.
        self.thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: actor.user_id.as_str().to_string(),
                title: None,
                metadata_json: None,
            })
            .await?;

        let source_binding_id = webui_binding_id(&actor);
        let accepted = self
            .thread_service
            .accept_inbound_message(ironclaw_threads::AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
                actor_id: actor.user_id.as_str().to_string(),
                source_binding_id: Some(source_binding_id.clone()),
                reply_target_binding_id: Some(source_binding_id.clone()),
                external_event_id: Some(client_action_id.as_str().to_string()),
                content: MessageContent::text(content),
            })
            .await?;

        let accepted_message_ref = accepted_message_ref(accepted.message_id)?;
        let source_binding_ref = build_source_binding_ref(&source_binding_id)?;
        let reply_target_binding_ref = build_reply_target_binding_ref(&source_binding_id)?;
        let received_at = chrono::Utc::now();

        let request = SubmitTurnRequest {
            scope: scope.clone(),
            actor,
            accepted_message_ref: accepted_message_ref.clone(),
            source_binding_ref,
            reply_target_binding_ref,
            requested_run_profile: None,
            idempotency_key: client_action_id,
            received_at,
        };

        match self.turn_coordinator.submit_turn(request).await {
            Ok(SubmitTurnResponse::Accepted {
                turn_id, run_id, ..
            }) => {
                self.thread_service
                    .mark_message_submitted(
                        &thread_scope,
                        &thread_id,
                        accepted.message_id,
                        turn_id.to_string(),
                        run_id.to_string(),
                    )
                    .await?;
                Ok(WebUiMessageAccepted {
                    thread_id,
                    accepted_message_ref,
                    run: WebUiMessageRunOutcome::Submitted { run_id },
                })
            }
            Err(TurnError::ThreadBusy(busy)) => {
                self.thread_service
                    .mark_message_deferred_busy(&thread_scope, &thread_id, accepted.message_id)
                    .await?;
                Ok(WebUiMessageAccepted {
                    thread_id,
                    accepted_message_ref,
                    run: WebUiMessageRunOutcome::DeferredBusy {
                        active_run_id: busy.active_run_id,
                    },
                })
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn cancel_run(
        &self,
        command: WebUiCancelRunCommand,
    ) -> Result<WebUiRunCancelled, WebUiServiceError> {
        let WebUiCancelRunCommand {
            scope,
            actor,
            run_id,
            reason,
            client_action_id,
        } = command;

        let response = self
            .turn_coordinator
            .cancel_run(CancelRunRequest {
                scope,
                actor,
                run_id,
                reason,
                idempotency_key: client_action_id,
            })
            .await?;

        Ok(WebUiRunCancelled {
            run_id: response.run_id,
            already_terminal: response.already_terminal,
        })
    }

    async fn resolve_gate(
        &self,
        command: WebUiResolveGateCommand,
    ) -> Result<WebUiGateResolved, WebUiServiceError> {
        let WebUiResolveGateCommand {
            scope,
            actor,
            run_id,
            gate_ref,
            client_action_id,
            resolution,
        } = command;

        match resolution {
            WebUiGateResolution::Approved { .. } => {
                let source_binding_id = webui_binding_id(&actor);
                let source_binding_ref = build_source_binding_ref(&source_binding_id)?;
                let reply_target_binding_ref = build_reply_target_binding_ref(&source_binding_id)?;
                let response = self
                    .turn_coordinator
                    .resume_turn(ResumeTurnRequest {
                        scope,
                        actor,
                        run_id,
                        gate_resolution_ref: gate_ref,
                        source_binding_ref,
                        reply_target_binding_ref,
                        idempotency_key: client_action_id,
                    })
                    .await?;
                Ok(WebUiGateResolved::Resumed {
                    run_id: response.run_id,
                })
            }
            // C2 fix: previously this arm fell through to `resume_turn` and
            // silently dropped `credential_ref`. The run would resume with no
            // credential actually bound, and the next tool call would either
            // re-trigger the auth gate or fail with a missing-credential
            // error. The credential-binding port that would make this
            // resolution honest does not exist in product_workflow yet, so
            // we fail loud with a typed Unsupported error rather than lie.
            WebUiGateResolution::CredentialProvided { .. } => Err(WebUiServiceError::Unsupported {
                what: "credential_provided_gate_resolution",
            }),
            WebUiGateResolution::Denied | WebUiGateResolution::Cancelled => {
                let response = self
                    .turn_coordinator
                    .cancel_run(CancelRunRequest {
                        scope,
                        actor,
                        run_id,
                        reason: SanitizedCancelReason::UserRequested,
                        idempotency_key: client_action_id,
                    })
                    .await?;
                Ok(WebUiGateResolved::Cancelled {
                    run_id: response.run_id,
                    already_terminal: response.already_terminal,
                })
            }
        }
    }

    async fn get_timeline_snapshot(
        &self,
        command: WebUiGetTimelineCommand,
    ) -> Result<WebUiTimelineSnapshot, WebUiServiceError> {
        let request = build_projection_request(&command)?;
        let snapshot = self.projection_service.snapshot(request).await?;
        Ok(WebUiTimelineSnapshot {
            entries: snapshot.timeline.entries,
            runs: snapshot.runs,
            next_cursor: WebUiTimelineCursor::from_projection(snapshot.next_cursor),
            truncated: snapshot.truncated,
        })
    }

    async fn get_timeline_updates(
        &self,
        command: WebUiGetTimelineCommand,
    ) -> Result<WebUiTimelineReplay, WebUiServiceError> {
        let request = build_projection_request(&command)?;
        let replay = self.projection_service.updates(request).await?;
        Ok(WebUiTimelineReplay {
            entries: replay.updates,
            runs: replay.runs,
            next_cursor: WebUiTimelineCursor::from_projection(replay.next_cursor),
            truncated: replay.truncated,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Derive a deterministic `ThreadId` from the browser-supplied idempotency
/// key. Same `client_action_id` → same thread_id, so `ensure_thread` makes
/// the retry path a no-op instead of creating a duplicate row.
fn derive_webui_thread_id(
    client_action_id: &IdempotencyKey,
) -> Result<ThreadId, WebUiServiceError> {
    let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, client_action_id.as_str().as_bytes());
    ThreadId::new(format!("thread:webui:{id}")).map_err(|_| WebUiServiceError::InvalidInput)
}

fn webui_thread_scope(caller: &WebUiAuthenticatedCaller) -> Result<ThreadScope, WebUiServiceError> {
    let Some(agent_id) = caller.agent_id.clone() else {
        return Err(WebUiServiceError::MissingAgentContext);
    };
    Ok(ThreadScope {
        tenant_id: caller.tenant_id.clone(),
        agent_id,
        project_id: caller.project_id.clone(),
        owner_user_id: Some(caller.user_id.clone()),
        mission_id: None,
    })
}

fn thread_scope_from_turn_scope(
    scope: &TurnScope,
    actor: &TurnActor,
) -> Result<ThreadScope, WebUiServiceError> {
    let Some(agent_id) = scope.agent_id.clone() else {
        return Err(WebUiServiceError::MissingAgentContext);
    };
    Ok(ThreadScope {
        tenant_id: scope.tenant_id.clone(),
        agent_id,
        project_id: scope.project_id.clone(),
        owner_user_id: Some(actor.user_id.clone()),
        mission_id: None,
    })
}

fn webui_binding_id(actor: &TurnActor) -> String {
    format!("webui:{}", actor.user_id.as_str())
}

fn accepted_message_ref(
    message_id: ironclaw_threads::ThreadMessageId,
) -> Result<AcceptedMessageRef, WebUiServiceError> {
    AcceptedMessageRef::new(format!("msg:{message_id}"))
        .map_err(|_| WebUiServiceError::InvalidInput)
}

fn build_source_binding_ref(value: &str) -> Result<SourceBindingRef, WebUiServiceError> {
    bounded_binding_ref(value)
        .and_then(|v| SourceBindingRef::new(v).map_err(|_| WebUiServiceError::InvalidInput))
}

fn build_reply_target_binding_ref(value: &str) -> Result<ReplyTargetBindingRef, WebUiServiceError> {
    bounded_binding_ref(value)
        .and_then(|v| ReplyTargetBindingRef::new(v).map_err(|_| WebUiServiceError::InvalidInput))
}

/// Bound a binding-ref string to a length the typed ref accepts. Long values
/// hash to a deterministic UUIDv5 so the ref is still stable per caller.
fn bounded_binding_ref(value: &str) -> Result<String, WebUiServiceError> {
    if value.len() <= 240 && !value.chars().any(|c| c == '\0' || c.is_control()) {
        Ok(value.to_string())
    } else {
        Ok(format!(
            "webui:{}",
            Uuid::new_v5(&Uuid::NAMESPACE_OID, value.as_bytes())
        ))
    }
}

fn build_projection_request(
    command: &WebUiGetTimelineCommand,
) -> Result<ProjectionRequest, WebUiServiceError> {
    let Some(agent_id) = command.caller.agent_id.clone() else {
        return Err(WebUiServiceError::MissingAgentContext);
    };
    let stream = EventStreamKey::new(
        command.caller.tenant_id.clone(),
        command.caller.user_id.clone(),
        Some(agent_id),
    );
    let read_scope = ReadScope {
        project_id: command.caller.project_id.clone(),
        mission_id: None,
        thread_id: Some(command.thread_id.clone()),
        process_id: None,
    };
    let scope = ProjectionScope { stream, read_scope };

    if let Some(cursor) = &command.after {
        // Defense in depth: the cursor must match the caller's scope.
        // The projection service also re-checks this, but rejecting early
        // keeps the error surface to typed `InvalidInput` rather than the
        // projection's stringly-typed rebase-required.
        if cursor.as_projection().scope != scope {
            return Err(WebUiServiceError::InvalidInput);
        }
    }

    let limit = clamp_timeline_limit(command.limit);
    Ok(ProjectionRequest {
        scope,
        after: command
            .after
            .clone()
            .map(WebUiTimelineCursor::into_projection),
        limit,
    })
}

/// Clamp the caller-supplied limit into `[1, MAX_PROJECTION_PAGE_LIMIT]`, using
/// [`WEBUI_TIMELINE_DEFAULT_LIMIT`] when the caller passes `0`.
fn clamp_timeline_limit(requested: usize) -> usize {
    let normalized = if requested == 0 {
        WEBUI_TIMELINE_DEFAULT_LIMIT
    } else {
        requested
    };
    normalized.min(MAX_PROJECTION_PAGE_LIMIT)
}
