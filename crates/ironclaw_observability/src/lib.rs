//! Shared low-level observability helpers.
#![warn(unreachable_pub)]

use std::time::Instant;

#[inline]
pub fn elapsed_ms(started_at: Instant) -> u64 {
    started_at
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[inline]
pub fn live_latency_enabled() -> bool {
    tracing::enabled!(target: "ironclaw_latency", tracing::Level::TRACE)
}

#[inline]
pub fn live_latency_started_at() -> Option<Instant> {
    live_latency_enabled().then(Instant::now)
}

#[macro_export]
macro_rules! live_latency_trace {
    ($($fields:tt)*) => {
        tracing::trace!(target: "ironclaw_latency", $($fields)*)
    };
}
