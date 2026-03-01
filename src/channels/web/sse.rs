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

use crate::channels::web::types::SseEvent;

/// Maximum number of concurrent SSE/WebSocket connections.
/// Prevents resource exhaustion from connection flooding.
const MAX_CONNECTIONS: u64 = 100;

/// Envelope for broadcast events: carries an optional user scope.
///
/// `user_id = None` means the event is global (e.g. Heartbeat) and delivered
/// to all subscribers. `user_id = Some(id)` means the event is only delivered
/// to subscribers that match that user_id.
#[derive(Debug, Clone)]
struct ScopedEvent {
    user_id: Option<String>,
    event: SseEvent,
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
    max_connections: u64,
}

impl SseManager {
    /// Create a new SSE manager.
    pub fn new() -> Self {
        // Buffer 256 events; slow clients will miss events (acceptable for SSE with reconnect)
        let (tx, _) = broadcast::channel(256);
        Self {
            tx,
            connection_count: Arc::new(AtomicU64::new(0)),
            max_connections: MAX_CONNECTIONS,
        }
    }

    /// Broadcast an event to all connected clients (global/unscoped).
    pub fn broadcast(&self, event: SseEvent) {
        let _ = self.tx.send(ScopedEvent {
            user_id: None,
            event,
        });
    }

    /// Broadcast an event scoped to a specific user.
    ///
    /// Only subscribers for this user_id (or unscoped subscribers) will
    /// receive the event.
    pub fn broadcast_for_user(&self, user_id: &str, event: SseEvent) {
        let _ = self.tx.send(ScopedEvent {
            user_id: Some(user_id.to_string()),
            event,
        });
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
    ) -> Option<impl Stream<Item = SseEvent> + Send + 'static + use<>> {
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
                    (_, None) => Some(scoped.event),                          // global -> all
                    (None, _) => Some(scoped.event),                          // unscoped subscriber -> all
                    (Some(sub), Some(ev)) if sub == ev => Some(scoped.event), // match
                    _ => None,                                                // different user -> skip
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
                    (_, None) => Some(scoped.event),
                    (None, _) => Some(scoped.event),
                    (Some(sub), Some(ev)) if sub == ev => Some(scoped.event),
                    _ => None,
                },
                Err(_) => None,
            })
            .map(|event| {
                let data = serde_json::to_string(&event).unwrap_or_default();
                let event_type = match &event {
                    SseEvent::Response { .. } => "response",
                    SseEvent::Thinking { .. } => "thinking",
                    SseEvent::ToolStarted { .. } => "tool_started",
                    SseEvent::ToolCompleted { .. } => "tool_completed",
                    SseEvent::ToolResult { .. } => "tool_result",
                    SseEvent::StreamChunk { .. } => "stream_chunk",
                    SseEvent::Status { .. } => "status",
                    SseEvent::ApprovalNeeded { .. } => "approval_needed",
                    SseEvent::AuthRequired { .. } => "auth_required",
                    SseEvent::AuthCompleted { .. } => "auth_completed",
                    SseEvent::Error { .. } => "error",
                    SseEvent::JobStarted { .. } => "job_started",
                    SseEvent::JobMessage { .. } => "job_message",
                    SseEvent::JobToolUse { .. } => "job_tool_use",
                    SseEvent::JobToolResult { .. } => "job_tool_result",
                    SseEvent::JobStatus { .. } => "job_status",
                    SseEvent::JobResult { .. } => "job_result",
                    SseEvent::Heartbeat => "heartbeat",
                    SseEvent::ExtensionStatus { .. } => "extension_status",
                };
                Ok(Event::default().event(event_type).data(data))
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
    }

    #[test]
    fn test_broadcast_without_receivers() {
        let manager = SseManager::new();
        // Should not panic even with no receivers
        manager.broadcast(SseEvent::Heartbeat);
    }

    #[tokio::test]
    async fn test_broadcast_to_receiver() {
        let manager = SseManager::new();
        let mut stream = Box::pin(manager.subscribe_raw(None).expect("should subscribe"));

        manager.broadcast(SseEvent::Status {
            message: "test".to_string(),
            thread_id: None,
        });

        let event = stream.next().await.unwrap();
        match event {
            SseEvent::Status { message, .. } => assert_eq!(message, "test"),
            _ => panic!("unexpected event type"),
        }
    }

    #[tokio::test]
    async fn test_subscribe_raw_receives_events() {
        let manager = SseManager::new();
        let mut stream = Box::pin(manager.subscribe_raw(None).expect("should subscribe"));

        assert_eq!(manager.connection_count(), 1);

        manager.broadcast(SseEvent::Thinking {
            message: "working".to_string(),
            thread_id: None,
        });

        let event = stream.next().await.unwrap();
        match event {
            SseEvent::Thinking { message, .. } => assert_eq!(message, "working"),
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

        manager.broadcast(SseEvent::Heartbeat);

        let e1 = s1.next().await.unwrap();
        let e2 = s2.next().await.unwrap();
        assert!(matches!(e1, SseEvent::Heartbeat));
        assert!(matches!(e2, SseEvent::Heartbeat));

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
        assert!(manager.subscribe(None).is_none());
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
            SseEvent::Status {
                message: "alice only".to_string(),
                thread_id: None,
            },
        );

        // Send global event
        manager.broadcast(SseEvent::Heartbeat);

        // Alice gets her scoped event
        let e = alice.next().await.unwrap();
        assert!(matches!(e, SseEvent::Status { .. }));

        // Alice also gets the global heartbeat
        let e = alice.next().await.unwrap();
        assert!(matches!(e, SseEvent::Heartbeat));

        // Bob only gets the global heartbeat (alice's event was filtered)
        let e = bob.next().await.unwrap();
        assert!(matches!(e, SseEvent::Heartbeat));
    }
}
