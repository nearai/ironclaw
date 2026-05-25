use std::sync::Arc;

use ironclaw_product_adapters::{
    AuthPromptView, GatePromptView, ProductAdapterError, ProductOutboundPayload,
    ProductProjectionItem, ProductProjectionState, ProductWorkflowRejectionKind, RedactedString,
};
use ironclaw_turns::{
    GetRunStateRequest, TurnCoordinator, TurnError, TurnEventKind, TurnEventProjectionCursor,
    TurnEventProjectionError, TurnEventProjectionRequest, TurnEventProjectionService,
    TurnEventProjectionSource, TurnLifecycleEvent, TurnScope, TurnStatus,
};

pub(super) const WEBUI_TURN_EVENT_PAGE_LIMIT: usize = 256;

pub(super) struct TurnEventPayload {
    pub(super) cursor: TurnEventProjectionCursor,
    pub(super) payload: ProductOutboundPayload,
}

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
    },
}

impl TurnEventBridge {
    pub(super) fn enabled(
        source: Arc<dyn TurnEventProjectionSource>,
        coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        Self::Enabled {
            service: Arc::new(TurnEventProjectionService::new(source)),
            coordinator,
        }
    }

    pub(super) async fn drain(
        &self,
        scope: &TurnScope,
        after: Option<TurnEventProjectionCursor>,
    ) -> Result<TurnEventDrain, ProductAdapterError> {
        let Self::Enabled {
            service,
            coordinator,
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
            for event in page.entries {
                if let Some(payload) = turn_event_payload(coordinator.as_ref(), &event).await? {
                    payloads.push(TurnEventPayload {
                        cursor: TurnEventProjectionCursor::for_scope(
                            event.scope.clone(),
                            event.cursor,
                        ),
                        payload,
                    });
                }
            }
            if !payloads.is_empty()
                || !page.truncated
                || after_cursor.as_ref() == Some(&page.next_cursor)
            {
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

async fn turn_event_payload(
    coordinator: &dyn TurnCoordinator,
    event: &TurnLifecycleEvent,
) -> Result<Option<ProductOutboundPayload>, ProductAdapterError> {
    if matches!(event.kind, TurnEventKind::Blocked)
        && let Some(prompt) = blocked_prompt_payload(coordinator, event).await?
    {
        return Ok(Some(prompt));
    }
    if projects_run_status(&event.kind) {
        return Ok(Some(ProductOutboundPayload::ProjectionUpdate {
            state: turn_event_projection_state(event)?,
        }));
    }
    Ok(None)
}

async fn blocked_prompt_payload(
    coordinator: &dyn TurnCoordinator,
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
    let Some(gate_ref) = state.gate_ref else {
        return Ok(None);
    };
    let gate_ref = gate_ref.as_str().to_string();
    match event.status {
        TurnStatus::BlockedAuth => Ok(Some(ProductOutboundPayload::AuthPrompt(AuthPromptView {
            turn_run_id: event.run_id,
            auth_request_ref: gate_ref,
            headline: "Authentication required".to_string(),
            body: event
                .sanitized_reason
                .clone()
                .unwrap_or_else(|| "Authenticate to continue this run.".to_string()),
        }))),
        TurnStatus::BlockedApproval => Ok(Some(gate_prompt(event, gate_ref, "Approval required"))),
        TurnStatus::BlockedResource => {
            Ok(Some(gate_prompt(event, gate_ref, "Resource unavailable")))
        }
        _ => Ok(None),
    }
}

fn gate_prompt(
    event: &TurnLifecycleEvent,
    gate_ref: String,
    headline: &'static str,
) -> ProductOutboundPayload {
    ProductOutboundPayload::GatePrompt(GatePromptView {
        turn_run_id: event.run_id,
        gate_ref,
        headline: headline.to_string(),
        body: event
            .sanitized_reason
            .clone()
            .unwrap_or_else(|| "Resolve this gate to continue the run.".to_string()),
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
) -> Result<ProductProjectionState, ProductAdapterError> {
    ProductProjectionState::new(
        event.scope.thread_id.to_string(),
        vec![ProductProjectionItem::RunStatus {
            run_id: event.run_id,
            status: turn_status_wire(event.status).to_string(),
        }],
    )
}

fn turn_status_wire(status: TurnStatus) -> &'static str {
    match status {
        TurnStatus::Queued => "queued",
        TurnStatus::Running => "running",
        TurnStatus::BlockedApproval => "blocked_approval",
        TurnStatus::BlockedAuth => "blocked_auth",
        TurnStatus::BlockedResource => "blocked_resource",
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
        operation = "map_projection_error",
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
