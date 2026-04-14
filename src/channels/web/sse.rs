//! SSE connection manager for broadcasting events to browser tabs.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::channels::web::types::AppEvent;

/// Maximum number of concurrent SSE/WebSocket connections.
/// Prevents resource exhaustion from connection flooding.
pub const DEFAULT_MAX_CONNECTIONS: u64 = 100;

/// Default broadcast buffer size. Under heavy tool use each tool call generates
/// 5-10 SSE events; a small buffer causes slow clients to lag and reconnect in
/// a cascade. Configurable via `SSE_BROADCAST_BUFFER` env var in `GatewayConfig`.
pub const DEFAULT_BROADCAST_BUFFER: usize = 1024;

/// Envelope for broadcast events: carries an optional user scope.
///
/// `user_id = None` means the event is global (e.g. Heartbeat) and delivered
/// to all subscribers. `user_id = Some(id)` means the event is only delivered
/// to subscribers that match that user_id.
#[derive(Debug, Clone)]
pub(crate) struct ScopedEvent {
    pub(crate) id: String,
    pub(crate) user_id: Option<String>,
    pub(crate) event: AppEvent,
}

/// Manages SSE broadcast to all connected browser tabs.
///
/// In multi-user mode, events are scoped by user_id so that each subscriber
/// only receives events intended for their user (plus global events like
/// Heartbeat). In single-user mode, all events are delivered to all subscribers
/// (backwards compatible).
pub struct SseManager {
    tx: broadcast::Sender<ScopedEvent>,
    connection_count: Arc<AtomicU64>,
    boot_id: Arc<str>,
    next_event_id: Arc<AtomicU64>,
    max_connections: u64,
}

impl SseManager {
    /// Create a new SSE manager with default settings.
    pub fn new() -> Self {
        Self::with_max_connections_and_buffer(DEFAULT_MAX_CONNECTIONS, DEFAULT_BROADCAST_BUFFER)
    }

    /// Create a new SSE manager with a custom connection limit and buffer size.
    pub fn with_max_connections_and_buffer(max_connections: u64, broadcast_buffer: usize) -> Self {
        let (tx, _) = broadcast::channel(broadcast_buffer);
        Self {
            tx,
            connection_count: Arc::new(AtomicU64::new(0)),
            boot_id: Arc::<str>::from(Uuid::new_v4().to_string()),
            next_event_id: Arc::new(AtomicU64::new(1)),
            max_connections,
        }
    }

    /// Create an SSE manager that reuses an existing broadcast sender.
    ///
    /// This preserves the broadcast channel across `rebuild_state` calls so
    /// that sender handles captured by other components remain valid.
    /// The broadcast channel capacity is already baked into `tx` at creation
    /// time; callers cannot change it by passing a new buffer size.
    ///
    /// **Important:** The connection counter is reset to zero and a fresh
    /// `boot_id` is generated (resetting the event-ID sequence). This method
    /// must only be called before the server starts accepting connections
    /// (i.e., during startup wiring). Calling it after connections are
    /// established will break connection tracking, allow exceeding
    /// `max_connections`, and invalidate event-ID dedup for connected clients.
    pub(crate) fn from_sender(tx: broadcast::Sender<ScopedEvent>, max_connections: u64) -> Self {
        Self {
            tx,
            connection_count: Arc::new(AtomicU64::new(0)),
            boot_id: Arc::<str>::from(Uuid::new_v4().to_string()),
            next_event_id: Arc::new(AtomicU64::new(1)),
            max_connections,
        }
    }

    /// Get a clone of the broadcast sender for use by other components.
    pub(crate) fn sender(&self) -> broadcast::Sender<ScopedEvent> {
        self.tx.clone()
    }

    /// Get the configured connection limit.
    pub fn max_connections(&self) -> u64 {
        self.max_connections
    }

    fn next_scoped_event(&self, user_id: Option<String>, event: AppEvent) -> ScopedEvent {
        let seq = self.next_event_id.fetch_add(1, Ordering::Relaxed);
        ScopedEvent {
            id: format!("{}:{seq}", self.boot_id),
            user_id,
            event,
        }
    }

    /// Broadcast an event to all connected clients (global/unscoped).
    pub fn broadcast(&self, event: AppEvent) {
        let _ = self.tx.send(self.next_scoped_event(None, event));
    }

    /// Broadcast an event scoped to a specific user.
    ///
    /// Only subscribers for this user_id (or unscoped subscribers) will
    /// receive the event.
    pub fn broadcast_for_user(&self, user_id: &str, event: AppEvent) {
        let _ = self
            .tx
            .send(self.next_scoped_event(Some(user_id.to_string()), event));
    }

    /// Get current number of active connections.
    pub fn connection_count(&self) -> u64 {
        self.connection_count.load(Ordering::Relaxed)
    }

    /// Create a raw broadcast subscription for non-SSE consumers (e.g. WebSocket).
    ///
    /// When `user_id` is `Some`, only events scoped to that user (or global
    /// events) are delivered. When `None`, all events are delivered (single-user
    /// backwards compatibility).
    ///
    /// Returns `None` if the maximum connection limit has been reached.
    pub fn subscribe_raw(
        &self,
        user_id: Option<String>,
    ) -> Option<impl Stream<Item = AppEvent> + Send + 'static + use<>> {
        // Atomically increment only if below the limit. This prevents
        // concurrent callers from overshooting max_connections.
        let counter = Arc::clone(&self.connection_count);
        let max = self.max_connections;
        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                if current < max {
                    Some(current + 1)
                } else {
                    None
                }
            })
            .ok()?;
        let rx = self.tx.subscribe();

        let stream = BroadcastStream::new(rx).filter_map(move |result| match result {
            Ok(scoped) => {
                // Global events (user_id=None) always pass through.
                // Scoped events only pass if the subscriber matches (or subscriber is unscoped).
                match (&user_id, &scoped.user_id) {
                    (_, None) => Some(scoped.event), // global -> all
                    (None, _) => Some(scoped.event), // unscoped subscriber -> all
                    (Some(sub), Some(ev)) if sub == ev => Some(scoped.event), // match
                    _ => None,                       // different user -> skip
                }
            }
            Err(_) => None,
        });

        Some(CountedStream {
            inner: stream,
            counter,
        })
    }

    /// Create a new SSE stream for a client connection.
    ///
    /// When `user_id` is `Some`, only events for that user (or global events)
    /// are delivered. When `None`, all events are delivered.
    ///
    /// Returns `None` if the maximum connection limit has been reached.
    pub fn subscribe(
        &self,
        user_id: Option<String>,
        last_event_id: Option<String>,
    ) -> Option<Sse<impl Stream<Item = Result<Event, Infallible>> + Send + 'static + use<>>> {
        // Atomically increment only if below the limit.
        let counter = Arc::clone(&self.connection_count);
        let max = self.max_connections;
        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                if current < max {
                    Some(current + 1)
                } else {
                    None
                }
            })
            .ok()?;
        let rx = self.tx.subscribe();

        let stream = BroadcastStream::new(rx)
            .filter_map(move |result| match result {
                Ok(scoped) => match (&user_id, &scoped.user_id) {
                    (_, None) => Some(scoped),
                    (None, _) => Some(scoped),
                    (Some(sub), Some(ev)) if sub == ev => Some(scoped),
                    _ => None,
                },
                Err(_) => None,
            })
            .filter_map(move |scoped| {
                if !is_event_after(last_event_id.as_deref(), &scoped.id) {
                    return None;
                }
                let data = match serde_json::to_string(&scoped.event) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("Failed to serialize SSE event: {}", e);
                        return None;
                    }
                };
                let event_type = scoped.event.event_type();
                Some(Ok(Event::default()
                    .id(scoped.id)
                    .event(event_type)
                    .data(data)))
            });

        // Wrap in a stream that decrements on drop
        let counted_stream = CountedStream {
            inner: stream,
            counter,
        };

        Some(
            Sse::new(counted_stream)
                .keep_alive(KeepAlive::new().interval(Duration::from_secs(30)).text("")),
        )
    }
}

fn parse_event_id(id: &str) -> Option<(&str, u64)> {
    let (boot_id, seq) = id.split_once(':')?;
    Some((boot_id, seq.parse().ok()?))
}

fn is_event_after(last_event_id: Option<&str>, current_event_id: &str) -> bool {
    let Some(last_event_id) = last_event_id else {
        return true;
    };
    let Some((last_boot_id, last_seq)) = parse_event_id(last_event_id) else {
        return true;
    };
    let Some((current_boot_id, current_seq)) = parse_event_id(current_event_id) else {
        return true;
    };
    if last_boot_id != current_boot_id {
        return true;
    }
    current_seq > last_seq
}

impl Default for SseManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Stream wrapper that decrements connection count on drop.
///
/// When the SSE client disconnects, this stream is dropped
/// and the counter is decremented.
struct CountedStream<S> {
    inner: S,
    counter: Arc<AtomicU64>,
}

impl<S: Stream + Unpin> Stream for CountedStream<S> {
    type Item = S::Item;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl<S> Drop for CountedStream<S> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_manager_creation() {
        let manager = SseManager::new();
        assert_eq!(manager.connection_count(), 0);
        assert_eq!(manager.max_connections(), DEFAULT_MAX_CONNECTIONS);
    }

    #[test]
    fn test_broadcast_without_receivers() {
        let manager = SseManager::new();
        // Should not panic even with no receivers
        manager.broadcast(AppEvent::Heartbeat);
    }

    #[tokio::test]
    async fn test_broadcast_to_receiver() {
        let manager = SseManager::new();
        let mut stream = Box::pin(manager.subscribe_raw(None).expect("should subscribe"));

        manager.broadcast(AppEvent::Status {
            message: "test".to_string(),
            thread_id: None,
        });

        let event = stream.next().await.unwrap();
        match event {
            AppEvent::Status { message, .. } => assert_eq!(message, "test"),
            _ => panic!("unexpected event type"),
        }
    }

    #[tokio::test]
    async fn test_subscribe_raw_receives_events() {
        let manager = SseManager::new();
        let mut stream = Box::pin(manager.subscribe_raw(None).expect("should subscribe"));

        assert_eq!(manager.connection_count(), 1);

        manager.broadcast(AppEvent::Thinking {
            message: "working".to_string(),
            thread_id: None,
        });

        let event = stream.next().await.unwrap();
        match event {
            AppEvent::Thinking { message, .. } => assert_eq!(message, "working"),
            _ => panic!("Expected Thinking event"),
        }
    }

    #[tokio::test]
    async fn test_subscribe_raw_decrements_on_drop() {
        let manager = SseManager::new();
        {
            let _stream = Box::pin(manager.subscribe_raw(None).expect("should subscribe"));
            assert_eq!(manager.connection_count(), 1);
        }
        // Stream dropped, counter should decrement
        assert_eq!(manager.connection_count(), 0);
    }

    #[tokio::test]
    async fn test_subscribe_raw_multiple_subscribers() {
        let manager = SseManager::new();
        let mut s1 = Box::pin(manager.subscribe_raw(None).expect("should subscribe"));
        let mut s2 = Box::pin(manager.subscribe_raw(None).expect("should subscribe"));
        assert_eq!(manager.connection_count(), 2);

        manager.broadcast(AppEvent::Heartbeat);

        let e1 = s1.next().await.unwrap();
        let e2 = s2.next().await.unwrap();
        assert!(matches!(e1, AppEvent::Heartbeat));
        assert!(matches!(e2, AppEvent::Heartbeat));

        drop(s1);
        assert_eq!(manager.connection_count(), 1);
        drop(s2);
        assert_eq!(manager.connection_count(), 0);
    }

    #[tokio::test]
    async fn test_subscribe_raw_rejects_over_limit() {
        let mut manager = SseManager::new();
        manager.max_connections = 2; // Low limit for testing

        let _s1 = Box::pin(manager.subscribe_raw(None).expect("first should succeed"));
        let _s2 = Box::pin(manager.subscribe_raw(None).expect("second should succeed"));
        assert_eq!(manager.connection_count(), 2);

        // Third should be rejected
        assert!(manager.subscribe_raw(None).is_none());
        assert!(manager.subscribe(None, None).is_none());
    }

    #[tokio::test]
    async fn test_scoped_events_filtered_by_user() {
        let manager = SseManager::new();
        let mut alice = Box::pin(
            manager
                .subscribe_raw(Some("alice".to_string()))
                .expect("subscribe"),
        );
        let mut bob = Box::pin(
            manager
                .subscribe_raw(Some("bob".to_string()))
                .expect("subscribe"),
        );

        // Send event scoped to alice
        manager.broadcast_for_user(
            "alice",
            AppEvent::Status {
                message: "alice only".to_string(),
                thread_id: None,
            },
        );

        // Send global event
        manager.broadcast(AppEvent::Heartbeat);

        // Alice gets her scoped event
        let e = alice.next().await.unwrap();
        assert!(matches!(e, AppEvent::Status { .. }));

        // Alice also gets the global heartbeat
        let e = alice.next().await.unwrap();
        assert!(matches!(e, AppEvent::Heartbeat));

        // Bob only gets the global heartbeat (alice's event was filtered)
        let e = bob.next().await.unwrap(); // safety: test-only
        assert!(matches!(e, AppEvent::Heartbeat)); // safety: test assertion
    }

    #[test]
    fn test_is_event_after_filters_same_boot_duplicates() {
        assert!(is_event_after(Some("boot:4"), "boot:5"));
        assert!(!is_event_after(Some("boot:5"), "boot:5"));
        assert!(!is_event_after(Some("boot:6"), "boot:5"));
    }

    #[test]
    fn test_is_event_after_ignores_other_boots_and_invalid_ids() {
        assert!(is_event_after(Some("old-boot:99"), "new-boot:1"));
        assert!(is_event_after(Some("not-an-id"), "new-boot:1"));
        assert!(is_event_after(Some("boot:1"), "also-bad"));
    }
}
