//! The session event stream: one `text/event-stream` per authenticated page
//! carrying every logical subscription named in the request body.
//!
//! The stream multiplexes independent typed subscriptions over one HTTP
//! response. It is an event transport, not a command bus: the client sends
//! nothing after the request body, nothing in this module calls
//! `ProductSurface::invoke` or dispatches an operation ID, and every product
//! mutation stays on authenticated HTTP. The subscription set is fixed for
//! the life of the connection; a client that needs a different set
//! reconnects with each selector's own resume cursor, which is the same
//! resume path it runs on lifetime expiry and on every dropped connection.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::Stream;
use tokio::sync::mpsc;

use ironclaw_product_contracts::outbound::ProjectionCursor;
use ironclaw_product_contracts::surface::{
    BoundProductSurface, ProductStreamSelector, ProductSurface, ProductSurfaceCaller,
    ProductSurfaceError, ProductSurfaceValidationCode,
};

use crate::webui_v2::error::WebUiV2HttpError;
use crate::webui_v2::handlers::{parse_cursor_token, sse_capacity_rejected};
use crate::webui_v2::router::WebUiV2State;
use crate::webui_v2::session_events::codec;
use crate::webui_v2::session_events::driver::{
    DriverStep, ProductStreamDriver, STREAM_KEEPALIVE_INTERVAL,
};
use crate::webui_v2::session_events::protocol::{
    SUBSCRIPTION_QUEUE_BATCHES, SessionEventsRequest, SessionServerFrame,
};
use crate::webui_v2::sse_capacity::{SSE_MAX_LIFETIME, SseAcquireResult, SseSlot};

/// `POST /api/webchat/v2/session/events`
///
/// Opens the page's session event stream. The bearer travels in the
/// `Authorization` header like every other route (the browser opens the
/// stream with `fetch`, not `EventSource`), so no transport-specific
/// credential exists. The stream shares the per-caller event-connection
/// budget with the compatibility per-thread SSE route.
pub async fn session_events(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Json(request): Json<SessionEventsRequest>,
) -> Result<Response, WebUiV2HttpError> {
    request.validate()?;
    let slot = match state
        .sse_capacity()
        .try_acquire(&caller.tenant_id, &caller.user_id, None)
    {
        SseAcquireResult::Acquired(slot) => slot,
        SseAcquireResult::AtCapacity { refundable } => {
            return Ok(sse_capacity_rejected(refundable));
        }
        SseAcquireResult::StaleGeneration => {
            return Ok(sse_capacity_rejected(false));
        }
    };
    let stream = session_event_stream(state.services().clone(), caller, request, slot);
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(STREAM_KEEPALIVE_INTERVAL))
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    Ok(response)
}

/// Everything one subscription task reports back to the stream writer.
enum SubscriptionEmit {
    Admitted {
        cursor: Option<String>,
    },
    Event {
        cursor: Option<String>,
        body: serde_json::Value,
    },
    Failed {
        error: ProductSurfaceError,
        last_cursor: Option<String>,
    },
    Ended,
}

/// One admitted logical subscription: its connection-scoped generation, the
/// queue its driver task fills, and the task itself, aborted on drop so a
/// client disconnect (the stream generator dropping) releases every driver.
struct SubscriptionEntry {
    generation: u64,
    receiver: mpsc::Receiver<SubscriptionEmit>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for SubscriptionEntry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run_subscription(
    surface: BoundProductSurface,
    selector: ProductStreamSelector,
    initial_cursor: Option<ProjectionCursor>,
    sender: mpsc::Sender<SubscriptionEmit>,
) {
    let admitted_cursor = initial_cursor
        .as_ref()
        .and_then(|cursor| serde_json::to_string(cursor).ok());
    let mut driver = ProductStreamDriver::new(surface, selector, initial_cursor);
    let mut last_forwarded = admitted_cursor.clone();
    match driver.open().await {
        Err(error) => {
            let _ = sender
                .send(SubscriptionEmit::Failed {
                    error,
                    last_cursor: last_forwarded,
                })
                .await;
            return;
        }
        Ok(first_events) => {
            if sender
                .send(SubscriptionEmit::Admitted {
                    cursor: admitted_cursor,
                })
                .await
                .is_err()
            {
                return;
            }
            for envelope in first_events {
                if !forward_stream_event(&sender, envelope, &mut last_forwarded).await {
                    return;
                }
            }
        }
    }
    loop {
        match driver.next_step().await {
            DriverStep::Events(events) => {
                for envelope in events {
                    if !forward_stream_event(&sender, envelope, &mut last_forwarded).await {
                        return;
                    }
                }
            }
            // The SSE layer's comment keep-alive proves liveness for the
            // whole stream; a quiet subscription needs no frame of its own.
            DriverStep::Idle => {}
            DriverStep::ServiceError(error) => {
                let _ = sender
                    .send(SubscriptionEmit::Failed {
                        error,
                        last_cursor: last_forwarded,
                    })
                    .await;
                return;
            }
            DriverStep::Ended => {
                let _ = sender.send(SubscriptionEmit::Ended).await;
                return;
            }
            DriverStep::LifetimeExpired => {
                let _ = sender
                    .send(SubscriptionEmit::Failed {
                        error: ProductSurfaceError::unavailable(true),
                        last_cursor: last_forwarded,
                    })
                    .await;
                return;
            }
        }
    }
}

/// Render one typed event through the shared browser codec and queue it,
/// advancing `last_forwarded` on success. Returns `false` when the stream
/// writer hung up.
///
/// A codec failure fails LOUD: the driver's cursor has already advanced past
/// the event, so silently continuing would hand the client later cursors and
/// make the dropped event unrecoverable on resume. Reporting `Failed` with
/// the last cursor the client actually received makes the client resubscribe
/// from a position that re-renders the event (or surfaces a persistent
/// serialization defect instead of hiding it).
async fn forward_stream_event(
    sender: &mpsc::Sender<SubscriptionEmit>,
    envelope: ironclaw_product_contracts::surface::ProductStreamEventEnvelope,
    last_forwarded: &mut Option<String>,
) -> bool {
    let rendered = codec::browser_frame(envelope)
        .map(|browser| (browser.cursor_token.clone(), browser.event_body()));
    let Some((cursor, body)) = rendered else {
        tracing::debug!(
            target: "ironclaw_webui_v2::session_events",
            "failing the subscription after an unserializable event",
        );
        let _ = sender
            .send(SubscriptionEmit::Failed {
                error: ProductSurfaceError::unavailable(true),
                last_cursor: last_forwarded.clone(),
            })
            .await;
        return false;
    };
    if let Some(cursor) = cursor.clone() {
        *last_forwarded = Some(cursor);
    }
    sender
        .send(SubscriptionEmit::Event { cursor, body })
        .await
        .is_ok()
}

/// Poll every subscription queue in rotation order so one hot subscription
/// cannot starve the others. Registers wakers on every pending queue;
/// returns `Poll::Pending` when nothing is ready.
fn poll_next_emit(
    active: &mut BTreeMap<String, SubscriptionEntry>,
    rotation: &mut usize,
    cx: &mut Context<'_>,
) -> Poll<(String, u64, SubscriptionEmit)> {
    let keys: Vec<String> = active.keys().cloned().collect();
    if keys.is_empty() {
        return Poll::Pending;
    }
    let count = keys.len();
    for offset in 0..count {
        let key = &keys[(*rotation + offset) % count];
        let Some(entry) = active.get_mut(key) else {
            continue;
        };
        match entry.receiver.poll_recv(cx) {
            Poll::Ready(Some(emit)) => {
                *rotation = (*rotation + offset + 1) % count;
                return Poll::Ready((key.clone(), entry.generation, emit));
            }
            Poll::Ready(None) => {
                *rotation = (*rotation + offset + 1) % count;
                return Poll::Ready((key.clone(), entry.generation, SubscriptionEmit::Ended));
            }
            Poll::Pending => {}
        }
    }
    Poll::Pending
}

/// Serialize one server frame as an SSE event named by its type. The frame
/// vocabulary is closed and total, so serialization cannot fail; the
/// fallback only guards a future frame carrying an unserializable value.
fn frame_event(frame: &SessionServerFrame) -> Option<Event> {
    match serde_json::to_string(frame) {
        Ok(text) => Some(Event::default().event(frame.name()).data(text)),
        Err(error) => {
            tracing::debug!(
                target: "ironclaw_webui_v2::session_events",
                error = %error,
                "failed to serialize session server frame",
            );
            None
        }
    }
}

/// The stream body: one driver task per requested subscription, frames
/// drained fairly, the per-connection lifetime budget, and the capacity slot
/// released when the generator drops (client disconnect, lifetime expiry, or
/// every subscription ending).
fn session_event_stream(
    services: Arc<dyn ProductSurface>,
    caller: ProductSurfaceCaller,
    request: SessionEventsRequest,
    slot: SseSlot,
) -> impl Stream<Item = Result<Event, Infallible>> {
    enum LoopStep {
        Cancelled,
        LifetimeExpired,
        Emit((String, u64, SubscriptionEmit)),
    }
    async_stream::stream! {
        let mut slot_guard = slot;
        let deadline = tokio::time::Instant::now() + SSE_MAX_LIFETIME;
        let mut active: BTreeMap<String, SubscriptionEntry> = BTreeMap::new();
        let mut rotation: usize = 0;
        for (index, subscription) in request.subscriptions.into_iter().enumerate() {
            let generation = index as u64 + 1;
            // A resume token the server never issued fails that subscription
            // loudly: silently resuming from the origin would replay the
            // whole history instead of "strictly after what the client saw".
            let initial_cursor = match subscription.after_cursor {
                None => None,
                Some(token) => match parse_cursor_token(token) {
                    Some(cursor) => Some(cursor),
                    None => {
                        let rejected = SessionServerFrame::subscription_error(
                            subscription.subscription_id,
                            generation,
                            &ProductSurfaceError::validation(
                                "after_cursor",
                                ProductSurfaceValidationCode::InvalidValue,
                            ),
                            None,
                        );
                        if let Some(event) = frame_event(&rejected) {
                            yield Ok(event);
                        }
                        continue;
                    }
                },
            };
            let (sender, receiver) = mpsc::channel(SUBSCRIPTION_QUEUE_BATCHES);
            let surface = BoundProductSurface::new(Arc::clone(&services), caller.clone());
            let task = tokio::spawn(run_subscription(
                surface,
                subscription.selector,
                initial_cursor,
                sender,
            ));
            active.insert(
                subscription.subscription_id,
                SubscriptionEntry {
                    generation,
                    receiver,
                    task,
                },
            );
        }
        while !active.is_empty() {
            let step = tokio::select! {
                biased;
                _ = slot_guard.cancelled() => LoopStep::Cancelled,
                _ = tokio::time::sleep_until(deadline) => LoopStep::LifetimeExpired,
                emit = std::future::poll_fn(|cx| {
                    poll_next_emit(&mut active, &mut rotation, cx)
                }) => LoopStep::Emit(emit),
            };
            let (subscription_id, generation, emit) = match step {
                LoopStep::Cancelled => return,
                LoopStep::LifetimeExpired => {
                    // Normal lifetime expiry: hint the client to reconnect
                    // (re-evaluating bearer validity) and resume each logical
                    // selector from its own cursor.
                    if let Some(event) = frame_event(&SessionServerFrame::lifetime_reconnect_hint()) {
                        yield Ok(event);
                    }
                    return;
                }
                LoopStep::Emit(emit) => emit,
            };
            let frame = match emit {
                SubscriptionEmit::Admitted { cursor } => {
                    SessionServerFrame::subscribed(subscription_id, generation, cursor)
                }
                SubscriptionEmit::Event { cursor, body } => {
                    SessionServerFrame::event(subscription_id, generation, cursor, body)
                }
                SubscriptionEmit::Failed { error, last_cursor } => {
                    // Only this subscription is released; the stream and
                    // every other subscription stay open.
                    active.remove(&subscription_id);
                    SessionServerFrame::subscription_error(
                        subscription_id,
                        generation,
                        &error,
                        last_cursor,
                    )
                }
                SubscriptionEmit::Ended => {
                    active.remove(&subscription_id);
                    SessionServerFrame::subscription_error(
                        subscription_id,
                        generation,
                        &ProductSurfaceError::unavailable(true),
                        None,
                    )
                }
            };
            if let Some(event) = frame_event(&frame) {
                yield Ok(event);
            }
        }
    }
}
