//! The shared product stream driver.
//!
//! One driver instance owns one logical `ProductSurface::stream_events`
//! subscription: the initial authorization-bearing drain, the continuous live
//! subscription, the per-connection lifetime budget, and the resume-cursor
//! advance. A surface that answers without a live subscription is a failed
//! subscription (the client resubscribes with backoff); the server never
//! polls on the client's behalf. Both the compatibility per-thread SSE
//! generator and each session-stream subscription task consume the driver
//! step-by-step, so the two transports share exactly one implementation of
//! resume and failure behavior.
//!
//! The one exception is the compatibility per-thread route's legacy idle
//! poll for drain-only surfaces ([`ProductStreamDriver::new_with_legacy_idle_polling`]),
//! kept only until that route and its API-client tests are retired.

use std::time::Duration;

use ironclaw_product_contracts::outbound::ProjectionCursor;
use ironclaw_product_contracts::surface::{
    BoundProductSurface, ProductStreamEventEnvelope, ProductStreamSelector, ProductSurfaceError,
    ProductSurfaceEventSubscription, ProductSurfaceStreamRequest,
};

use crate::webui_v2::sse_capacity::SSE_MAX_LIFETIME;

/// Legacy per-thread route only: base interval between service polls while a
/// drain-only stream is idle.
pub(crate) const STREAM_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Legacy per-thread route only: ceiling for the idle poll backoff.
pub(crate) const STREAM_IDLE_POLL_MAX_INTERVAL: Duration = Duration::from_secs(3);

/// Cadence for typed application keep-alive liveness proof while a live
/// subscription is legitimately quiet.
pub(crate) const STREAM_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Legacy per-thread route only: idle poll backoff, two fast polls, one
/// medium, then the ceiling.
pub(crate) fn stream_poll_interval_for_idle_polls(idle_polls: u32) -> Duration {
    match idle_polls {
        0 | 1 => STREAM_POLL_INTERVAL,
        2 => Duration::from_secs(2),
        _ => STREAM_IDLE_POLL_MAX_INTERVAL,
    }
}

/// One observable step of a logical product stream.
pub(crate) enum DriverStep {
    /// A non-empty batch of typed events, in delivery order.
    Events(Vec<ProductStreamEventEnvelope>),
    /// The live subscription has been quiet for one keep-alive interval.
    /// Transports that need liveness proof emit their keep-alive frame.
    Idle,
    /// The service rejected the stream; terminal for this subscription.
    ServiceError(ProductSurfaceError),
    /// The connection lifetime budget is exhausted; the transport closes and
    /// the client reconnects with its last cursor.
    LifetimeExpired,
    /// The service ended the subscription without an error.
    Ended,
}

pub(crate) struct ProductStreamDriver {
    surface: BoundProductSurface,
    selector: ProductStreamSelector,
    after_cursor: Option<ProjectionCursor>,
    subscription: Option<ProductSurfaceEventSubscription>,
    idle_polls: u32,
    /// Compatibility per-thread route only: poll a drain-only surface at the
    /// idle cadence instead of failing the subscription.
    legacy_idle_polling: bool,
    started_at: tokio::time::Instant,
    lifetime: Duration,
}

impl ProductStreamDriver {
    /// A live-only driver: the surface must hand back a continuous
    /// subscription, or the stream fails and the client resubscribes.
    pub(crate) fn new(
        surface: BoundProductSurface,
        selector: ProductStreamSelector,
        initial_cursor: Option<ProjectionCursor>,
    ) -> Self {
        Self {
            surface,
            selector,
            after_cursor: initial_cursor,
            subscription: None,
            idle_polls: 0,
            legacy_idle_polling: false,
            started_at: tokio::time::Instant::now(),
            lifetime: SSE_MAX_LIFETIME,
        }
    }

    /// The compatibility per-thread route's driver: a drain-only surface is
    /// polled at the legacy idle cadence. Retired with that route.
    pub(crate) fn new_with_legacy_idle_polling(
        surface: BoundProductSurface,
        selector: ProductStreamSelector,
        initial_cursor: Option<ProjectionCursor>,
    ) -> Self {
        Self {
            legacy_idle_polling: true,
            ..Self::new(surface, selector, initial_cursor)
        }
    }

    fn remaining(&self) -> Duration {
        self.lifetime.saturating_sub(self.started_at.elapsed())
    }

    fn record_batch_cursor(&mut self, events: &[ProductStreamEventEnvelope]) {
        if let Some(latest) = events.last() {
            self.after_cursor = Some(latest.cursor.clone());
        }
    }

    /// Perform the initial authorization-bearing drain.
    ///
    /// The session socket admits (or replaces) a logical subscription only
    /// after the product surface has authorized its selector, which happens
    /// on the first `stream_events` call. Returns the first event batch
    /// (possibly empty) on success so no event is lost between admission and
    /// the step loop.
    pub(crate) async fn open(
        &mut self,
    ) -> Result<Vec<ProductStreamEventEnvelope>, ProductSurfaceError> {
        let remaining = self.remaining();
        if remaining.is_zero() {
            return Err(ProductSurfaceError::unavailable(true));
        }
        let request = ProductSurfaceStreamRequest {
            selector: self.selector.clone(),
            after_cursor: self
                .after_cursor
                .as_ref()
                .map(|cursor| cursor.as_str().to_string()),
        };
        match tokio::time::timeout(remaining, self.surface.stream_events(request)).await {
            Err(_elapsed) => Err(ProductSurfaceError::unavailable(true)),
            Ok(Err(error)) => Err(error),
            Ok(Ok(mut response)) => {
                self.subscription = response.subscription.take();
                if self.subscription.is_none() && !self.legacy_idle_polling {
                    // No live continuation: nothing here would ever deliver
                    // a later event. Fail the subscription before the cursor
                    // advances so the client resubscribes from the same
                    // position and the drained events are not lost.
                    return Err(ProductSurfaceError::unavailable(true));
                }
                self.record_batch_cursor(&response.events);
                if !response.events.is_empty() {
                    self.idle_polls = 0;
                }
                Ok(response.events)
            }
        }
    }

    /// Drive the stream to its next observable step.
    ///
    /// Cancel-safe: dropping the returned future between awaits never loses
    /// delivered events (a response is only consumed in the same poll that
    /// returns it as a step).
    pub(crate) async fn next_step(&mut self) -> DriverStep {
        loop {
            let remaining = self.remaining();
            if remaining.is_zero() {
                return DriverStep::LifetimeExpired;
            }

            if let Some(subscription) = &self.subscription {
                tokio::select! {
                    biased;
                    next = tokio::time::timeout(remaining, subscription.next()) => {
                        return match next {
                            Ok(Some(Ok(response))) => {
                                self.record_batch_cursor(&response.events);
                                DriverStep::Events(response.events)
                            }
                            Ok(Some(Err(error))) => DriverStep::ServiceError(error),
                            Ok(None) => DriverStep::Ended,
                            Err(_elapsed) => DriverStep::LifetimeExpired,
                        };
                    }
                    _ = tokio::time::sleep(STREAM_KEEPALIVE_INTERVAL) => {
                        return DriverStep::Idle;
                    }
                }
            }

            if !self.legacy_idle_polling {
                // The live continuation ended without a terminal step (or
                // was never established): fail the subscription rather than
                // poll on the client's behalf.
                return DriverStep::ServiceError(ProductSurfaceError::unavailable(true));
            }
            let request = ProductSurfaceStreamRequest {
                selector: self.selector.clone(),
                after_cursor: self
                    .after_cursor
                    .as_ref()
                    .map(|cursor| cursor.as_str().to_string()),
            };
            let drained =
                tokio::time::timeout(remaining, self.surface.stream_events(request)).await;
            match drained {
                Err(_elapsed) => return DriverStep::LifetimeExpired,
                Ok(Err(error)) => return DriverStep::ServiceError(error),
                Ok(Ok(mut response)) => {
                    self.subscription = response.subscription.take();
                    self.record_batch_cursor(&response.events);
                    if !response.events.is_empty() {
                        self.idle_polls = 0;
                        return DriverStep::Events(response.events);
                    }
                    if self.subscription.is_some() {
                        // A live subscription opened with no first event;
                        // enter subscription mode without an idle sleep.
                        continue;
                    }
                    self.idle_polls = self.idle_polls.saturating_add(1);
                    let sleep_for =
                        stream_poll_interval_for_idle_polls(self.idle_polls).min(self.remaining());
                    if sleep_for.is_zero() {
                        return DriverStep::LifetimeExpired;
                    }
                    tokio::time::sleep(sleep_for).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_poll_interval_backs_off_only_after_repeated_idle_drains() {
        assert_eq!(stream_poll_interval_for_idle_polls(0), STREAM_POLL_INTERVAL);
        assert_eq!(stream_poll_interval_for_idle_polls(1), STREAM_POLL_INTERVAL);
        assert_eq!(
            stream_poll_interval_for_idle_polls(2),
            Duration::from_secs(2)
        );
        assert_eq!(
            stream_poll_interval_for_idle_polls(3),
            STREAM_IDLE_POLL_MAX_INTERVAL
        );
        assert_eq!(
            stream_poll_interval_for_idle_polls(u32::MAX),
            STREAM_IDLE_POLL_MAX_INTERVAL
        );
    }
}
