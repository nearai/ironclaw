//! Session event transport handlers: ticket minting and the app-wide
//! read-only session WebSocket.
//!
//! The socket multiplexes independent typed logical subscriptions over one
//! physical connection per authenticated page. It is an event transport,
//! not a command bus: client frames are limited to the closed
//! subscribe/unsubscribe/ping vocabulary in
//! [`super::super::session_events::protocol`], and nothing in this module
//! calls `ProductSurface::invoke` or dispatches an operation ID. Every
//! product mutation stays on authenticated HTTP.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use futures::SinkExt;
use serde::Serialize;
use tokio::sync::mpsc;

use ironclaw_product_contracts::outbound::ProjectionCursor;
use ironclaw_product_contracts::session_transport::{
    SESSION_SOCKET_TICKET_TTL_MS, SessionSocketTicket,
};
use ironclaw_product_contracts::surface::{
    BoundProductSurface, ProductStreamSelector, ProductSurface, ProductSurfaceCaller,
    ProductSurfaceError,
};

use super::super::error::WebUiV2HttpError;
use super::super::router::{WebUiV2Capabilities, WebUiV2State};
use super::super::session_events::codec;
use super::super::session_events::driver::{DriverStep, ProductStreamDriver};
use super::super::session_events::protocol::{
    MAX_ACTIVE_SUBSCRIPTIONS, SUBSCRIPTION_QUEUE_BATCHES, SessionClientFrame,
    SessionProtocolViolation, SessionServerFrame, parse_client_frame,
};
use super::super::sse_capacity::{SSE_MAX_LIFETIME, SseAcquireResult, SseSlot};
use super::{parse_cursor_token, sse_capacity_rejected};

/// Response body for `POST /api/webchat/v2/session/websocket-ticket`.
#[derive(Debug, Clone, Serialize)]
pub struct SessionSocketTicketResponse {
    /// Opaque single-use nonce; never identity or bearer material.
    pub ticket: String,
    /// Milliseconds until the ticket expires.
    pub expires_in_ms: u64,
    /// The socket path the ticket authenticates against.
    pub socket_path: &'static str,
}

/// `POST /api/webchat/v2/session/websocket-ticket`
///
/// Mints a bounded, single-use, short-lived socket ticket bound to the exact
/// authenticated caller. The opaque ticket is random and carries no identity
/// or bearer material; a logged ticket is already consumed or expires within
/// seconds.
pub async fn mint_session_socket_ticket(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<SessionSocketTicketResponse>, WebUiV2HttpError> {
    let Some(store) = state.session_socket_tickets() else {
        // No ticket store wired for this deployment shape: the session
        // socket capability is not advertised, and the mint route fails
        // closed for callers that probe it anyway.
        return Err(ProductSurfaceError::service_unavailable(false).into());
    };
    let ticket = SessionSocketTicket {
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
        operator_config: capabilities.operator_webui_config,
        expires_at_unix_ms: unix_now_ms().saturating_add(SESSION_SOCKET_TICKET_TTL_MS),
    };
    let nonce = store.mint(ticket).await.map_err(|error| {
        tracing::debug!(
            target: "ironclaw_webui_v2::session_socket",
            error = %error,
            "session socket ticket mint failed",
        );
        WebUiV2HttpError::from(ProductSurfaceError::unavailable(true))
    })?;
    Ok(Json(SessionSocketTicketResponse {
        ticket: nonce,
        expires_in_ms: SESSION_SOCKET_TICKET_TTL_MS,
        socket_path: super::super::descriptors::WEBUI_V2_PATTERN_SESSION_WEBSOCKET,
    }))
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// `GET /api/webchat/v2/session/websocket?ticket=<single-use-ticket>`
///
/// The bearer middleware consumed the single-use ticket and injected the
/// caller it bound; by the time this handler runs the request is
/// authenticated exactly like every other route. The upgrade shares the
/// per-caller event-connection budget with SSE.
pub async fn session_websocket(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    upgrade: WebSocketUpgrade,
) -> Result<axum::response::Response, WebUiV2HttpError> {
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
    let services = state.services().clone();
    Ok(upgrade.on_upgrade(move |socket| session_socket_loop(services, caller, slot, socket)))
}

/// Everything one subscription task reports back to the socket loop.
enum SubscriptionEmit {
    /// The selector authorized and the stream opened; `cursor` is the resume
    /// position after the initial drain (if any events were replayed).
    Admitted { cursor: Option<String> },
    /// One browser-safe event.
    Event {
        cursor: Option<String>,
        body: serde_json::Value,
    },
    /// Terminal failure; the subscription's generation is cancelled but the
    /// socket stays open.
    Failed {
        error: ProductSurfaceError,
        last_cursor: Option<String>,
    },
    /// The service ended the subscription without an error.
    Ended,
}

struct SubscriptionEntry {
    generation: u64,
    receiver: mpsc::Receiver<SubscriptionEmit>,
    task: tokio::task::JoinHandle<()>,
    admitted: bool,
}

impl SubscriptionEntry {
    fn cancel(self) {
        self.task.abort();
    }
}

/// Drive one logical subscription through the shared stream driver, feeding
/// its bounded queue. Dropping the receiver (entry removal, replacement, or
/// socket teardown) aborts this task's sends and ends it.
async fn run_subscription(
    surface: BoundProductSurface,
    selector: ProductStreamSelector,
    initial_cursor: Option<ProjectionCursor>,
    sender: mpsc::Sender<SubscriptionEmit>,
) {
    let mut driver = ProductStreamDriver::new(surface, selector, initial_cursor);
    match driver.open().await {
        Err(error) => {
            let last_cursor = cursor_token_of(driver.last_cursor());
            let _ = sender
                .send(SubscriptionEmit::Failed { error, last_cursor })
                .await;
            return;
        }
        Ok(first_events) => {
            if sender
                .send(SubscriptionEmit::Admitted {
                    cursor: cursor_token_of(driver.last_cursor()),
                })
                .await
                .is_err()
            {
                return;
            }
            for envelope in first_events {
                if !forward_stream_event(&sender, envelope).await {
                    return;
                }
            }
        }
    }
    loop {
        match driver.next_step().await {
            DriverStep::Events(events) => {
                for envelope in events {
                    if !forward_stream_event(&sender, envelope).await {
                        return;
                    }
                }
            }
            // The session socket has client pings for liveness; idle steps
            // need no server frame.
            DriverStep::Idle => {}
            DriverStep::ServiceError(error) => {
                let last_cursor = cursor_token_of(driver.last_cursor());
                let _ = sender
                    .send(SubscriptionEmit::Failed { error, last_cursor })
                    .await;
                return;
            }
            DriverStep::Ended => {
                let _ = sender.send(SubscriptionEmit::Ended).await;
                return;
            }
            DriverStep::LifetimeExpired => {
                // The per-subscription budget outlived the socket's own
                // lifetime frame; report a retryable interruption so the
                // client resumes from its cursor on the next socket.
                let last_cursor = cursor_token_of(driver.last_cursor());
                let _ = sender
                    .send(SubscriptionEmit::Failed {
                        error: ProductSurfaceError::unavailable(true),
                        last_cursor,
                    })
                    .await;
                return;
            }
        }
    }
}

fn cursor_token_of(cursor: Option<&ProjectionCursor>) -> Option<String> {
    cursor.and_then(|cursor| serde_json::to_string(cursor).ok())
}

/// Render one typed event through the shared browser codec and queue it.
/// Returns `false` when the socket side hung up.
async fn forward_stream_event(
    sender: &mpsc::Sender<SubscriptionEmit>,
    envelope: ironclaw_product_contracts::surface::ProductStreamEventEnvelope,
) -> bool {
    let Some(browser) = codec::browser_frame(envelope) else {
        tracing::debug!(
            target: "ironclaw_webui_v2::session_socket",
            "failed to serialize session event body",
        );
        return true;
    };
    let cursor = browser.cursor_token.clone();
    match browser.event_body() {
        Ok(body) => sender
            .send(SubscriptionEmit::Event { cursor, body })
            .await
            .is_ok(),
        Err(error) => {
            tracing::debug!(
                target: "ironclaw_webui_v2::session_socket",
                error = %error,
                "failed to serialize session event body",
            );
            true
        }
    }
}

/// Poll every subscription queue in rotation order, active before pending,
/// so one hot subscription cannot starve the others. Registers wakers on
/// every pending queue; returns `Poll::Pending` when nothing is ready.
fn poll_next_emit(
    active: &mut BTreeMap<String, SubscriptionEntry>,
    pending: &mut BTreeMap<String, SubscriptionEntry>,
    rotation: &mut usize,
    cx: &mut Context<'_>,
) -> Poll<(String, bool, u64, SubscriptionEmit)> {
    let keys: Vec<(String, bool)> = active
        .keys()
        .map(|key| (key.clone(), false))
        .chain(pending.keys().map(|key| (key.clone(), true)))
        .collect();
    if keys.is_empty() {
        return Poll::Pending;
    }
    let count = keys.len();
    for offset in 0..count {
        let (key, from_pending) = &keys[(*rotation + offset) % count];
        let entry = if *from_pending {
            pending.get_mut(key)
        } else {
            active.get_mut(key)
        };
        let Some(entry) = entry else { continue };
        match entry.receiver.poll_recv(cx) {
            Poll::Ready(Some(emit)) => {
                *rotation = (*rotation + offset + 1) % count;
                return Poll::Ready((key.clone(), *from_pending, entry.generation, emit));
            }
            Poll::Ready(None) => {
                *rotation = (*rotation + offset + 1) % count;
                return Poll::Ready((
                    key.clone(),
                    *from_pending,
                    entry.generation,
                    SubscriptionEmit::Ended,
                ));
            }
            Poll::Pending => {}
        }
    }
    Poll::Pending
}

async fn session_socket_loop(
    services: Arc<dyn ProductSurface>,
    caller: ProductSurfaceCaller,
    slot: SseSlot,
    mut socket: WebSocket,
) {
    let mut slot_guard = slot;
    let started_at = tokio::time::Instant::now();
    let deadline = started_at + SSE_MAX_LIFETIME;
    let mut active: BTreeMap<String, SubscriptionEntry> = BTreeMap::new();
    let mut pending: BTreeMap<String, SubscriptionEntry> = BTreeMap::new();
    let mut rotation: usize = 0;
    let mut next_generation: u64 = 0;

    loop {
        let has_subscriptions = !active.is_empty() || !pending.is_empty();
        enum LoopStep {
            Cancelled,
            LifetimeExpired,
            Inbound(Option<Result<Message, axum::Error>>),
            Emit((String, bool, u64, SubscriptionEmit)),
        }
        let step = tokio::select! {
            biased;
            _ = slot_guard.cancelled() => LoopStep::Cancelled,
            _ = tokio::time::sleep_until(deadline) => LoopStep::LifetimeExpired,
            inbound = socket.recv() => LoopStep::Inbound(inbound),
            emit = std::future::poll_fn(|cx| {
                poll_next_emit(&mut active, &mut pending, &mut rotation, cx)
            }), if has_subscriptions => LoopStep::Emit(emit),
        };
        match step {
            LoopStep::Cancelled => {
                cancel_all(active, pending);
                let _ = socket.close().await;
                return;
            }
            LoopStep::LifetimeExpired => {
                // Normal lifetime expiry: hint the client to mint a fresh
                // ticket (re-evaluating bearer validity) and resume each
                // logical selector from its own cursor.
                let _ = send_session_frame(
                    &mut socket,
                    &SessionServerFrame::lifetime_reconnect_hint(),
                    deadline,
                )
                .await;
                cancel_all(active, pending);
                let _ = socket.close().await;
                return;
            }
            LoopStep::Inbound(inbound) => match inbound {
                None | Some(Err(_)) => {
                    cancel_all(active, pending);
                    return;
                }
                Some(Ok(Message::Close(_))) => {
                    cancel_all(active, pending);
                    return;
                }
                Some(Ok(Message::Text(text))) => match parse_client_frame(text.as_str()) {
                    Ok(SessionClientFrame::Ping) => {
                        if send_session_frame(&mut socket, &SessionServerFrame::pong(), deadline)
                            .await
                            .is_err()
                        {
                            cancel_all(active, pending);
                            return;
                        }
                    }
                    Ok(SessionClientFrame::Unsubscribe { subscription_id }) => {
                        if let Some(entry) = pending.remove(&subscription_id) {
                            entry.cancel();
                        }
                        if let Some(entry) = active.remove(&subscription_id) {
                            let generation = entry.generation;
                            entry.cancel();
                            if send_session_frame(
                                &mut socket,
                                &SessionServerFrame::unsubscribed(subscription_id, generation),
                                deadline,
                            )
                            .await
                            .is_err()
                            {
                                cancel_all(active, pending);
                                return;
                            }
                        }
                    }
                    Ok(SessionClientFrame::Subscribe {
                        subscription_id,
                        selector,
                        after_cursor,
                    }) => {
                        let distinct = active.len() + pending.len()
                            - usize::from(pending.contains_key(&subscription_id));
                        let replaces_active = active.contains_key(&subscription_id);
                        if !replaces_active && distinct >= MAX_ACTIVE_SUBSCRIPTIONS {
                            let _ = send_session_frame(
                                &mut socket,
                                &SessionServerFrame::protocol_error(
                                    SessionProtocolViolation::TooManySubscriptions,
                                ),
                                deadline,
                            )
                            .await;
                            cancel_all(active, pending);
                            let _ = socket.close().await;
                            return;
                        }
                        next_generation += 1;
                        let (sender, receiver) = mpsc::channel(SUBSCRIPTION_QUEUE_BATCHES);
                        let surface =
                            BoundProductSurface::new(Arc::clone(&services), caller.clone());
                        let initial_cursor = after_cursor.and_then(parse_cursor_token);
                        let task = tokio::spawn(run_subscription(
                            surface,
                            selector,
                            initial_cursor,
                            sender,
                        ));
                        let entry = SubscriptionEntry {
                            generation: next_generation,
                            receiver,
                            task,
                            admitted: false,
                        };
                        // A replacement authorizes first: it stages in
                        // `pending` and swaps in only on `Admitted`, so an
                        // unauthorized replacement can never displace the
                        // existing authorized subscription.
                        if replaces_active {
                            if let Some(stale) = pending.insert(subscription_id, entry) {
                                stale.cancel();
                            }
                        } else if let Some(stale) = active.insert(subscription_id, entry) {
                            stale.cancel();
                        }
                    }
                    Err(violation) => {
                        let _ = send_session_frame(
                            &mut socket,
                            &SessionServerFrame::protocol_error(violation),
                            deadline,
                        )
                        .await;
                        cancel_all(active, pending);
                        let _ = socket.close().await;
                        return;
                    }
                },
                Some(Ok(Message::Binary(_))) => {
                    let _ = send_session_frame(
                        &mut socket,
                        &SessionServerFrame::protocol_error(
                            SessionProtocolViolation::BinaryFrameUnsupported,
                        ),
                        deadline,
                    )
                    .await;
                    cancel_all(active, pending);
                    let _ = socket.close().await;
                    return;
                }
                // Axum answers transport pings internally.
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            },
            LoopStep::Emit((subscription_id, from_pending, generation, emit)) => {
                let frame = if from_pending {
                    match emit {
                        SubscriptionEmit::Admitted { cursor } => {
                            // Authorized replacement: atomically swap
                            // generations. The displaced entry's queued
                            // frames drop with its receiver, so a stale
                            // generation can never deliver after the swap.
                            let Some(mut entry) = pending.remove(&subscription_id) else {
                                continue;
                            };
                            entry.admitted = true;
                            if let Some(stale) = active.insert(subscription_id.clone(), entry) {
                                stale.cancel();
                            }
                            Some(SessionServerFrame::subscribed(
                                subscription_id,
                                generation,
                                cursor,
                            ))
                        }
                        SubscriptionEmit::Failed { error, last_cursor } => {
                            // The attempted replacement is rejected and the
                            // existing authorized subscription continues
                            // unchanged.
                            if let Some(entry) = pending.remove(&subscription_id) {
                                entry.cancel();
                            }
                            Some(SessionServerFrame::subscription_error(
                                subscription_id,
                                generation,
                                &error,
                                last_cursor,
                            ))
                        }
                        SubscriptionEmit::Ended => {
                            if let Some(entry) = pending.remove(&subscription_id) {
                                entry.cancel();
                            }
                            Some(SessionServerFrame::subscription_error(
                                subscription_id,
                                generation,
                                &ProductSurfaceError::unavailable(true),
                                None,
                            ))
                        }
                        // A staged replacement buffers events behind its
                        // Admitted emit; they deliver after the swap.
                        SubscriptionEmit::Event { .. } => None,
                    }
                } else {
                    match emit {
                        SubscriptionEmit::Admitted { cursor } => {
                            if let Some(entry) = active.get_mut(&subscription_id) {
                                entry.admitted = true;
                            }
                            Some(SessionServerFrame::subscribed(
                                subscription_id,
                                generation,
                                cursor,
                            ))
                        }
                        SubscriptionEmit::Event { cursor, body } => Some(
                            SessionServerFrame::event(subscription_id, generation, cursor, body),
                        ),
                        SubscriptionEmit::Failed { error, last_cursor } => {
                            if let Some(entry) = active.remove(&subscription_id) {
                                entry.cancel();
                            }
                            Some(SessionServerFrame::subscription_error(
                                subscription_id,
                                generation,
                                &error,
                                last_cursor,
                            ))
                        }
                        SubscriptionEmit::Ended => {
                            if let Some(entry) = active.remove(&subscription_id) {
                                entry.cancel();
                            }
                            Some(SessionServerFrame::subscription_error(
                                subscription_id,
                                generation,
                                &ProductSurfaceError::unavailable(true),
                                None,
                            ))
                        }
                    }
                };
                if let Some(frame) = frame
                    && send_session_frame(&mut socket, &frame, deadline)
                        .await
                        .is_err()
                {
                    // Aggregate socket backpressure or peer hangup:
                    // close the connection and release everything.
                    cancel_all(active, pending);
                    return;
                }
            }
        }
    }
}

fn cancel_all(
    active: BTreeMap<String, SubscriptionEntry>,
    pending: BTreeMap<String, SubscriptionEntry>,
) {
    for (_, entry) in active {
        entry.cancel();
    }
    for (_, entry) in pending {
        entry.cancel();
    }
}

/// Serialize and send one session frame, bounded by the socket's remaining
/// lifetime so a stalled peer cannot pin the connection slot.
async fn send_session_frame(
    socket: &mut WebSocket,
    frame: &SessionServerFrame,
    deadline: tokio::time::Instant,
) -> Result<(), ()> {
    let text = match serde_json::to_string(frame) {
        Ok(text) => text,
        Err(error) => {
            tracing::debug!(
                target: "ironclaw_webui_v2::session_socket",
                error = %error,
                "failed to serialize session server frame",
            );
            return Ok(());
        }
    };
    let budget = deadline.saturating_duration_since(tokio::time::Instant::now());
    if budget.is_zero() {
        let _ = socket.close().await;
        return Err(());
    }
    match tokio::time::timeout(budget, socket.send(Message::Text(text.into()))).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(()),
        Err(_elapsed) => {
            tracing::debug!(
                target: "ironclaw_webui_v2::session_socket",
                "session frame send exceeded lifetime budget; releasing slot",
            );
            Err(())
        }
    }
}
