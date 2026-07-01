//! Shared low-level observability helpers.
#![warn(unreachable_pub)]

use std::time::Instant;

pub use tracing;

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
        $crate::tracing::trace!(target: "ironclaw_latency", $($fields)*)
    };
}

#[macro_export]
macro_rules! live_latency_trace_ok {
    ($component:expr, $operation:expr, $started_at:expr, $($fields:tt)*) => {
        if let Some(started_at) = $started_at {
            let elapsed_ms = $crate::elapsed_ms(started_at);
            $crate::live_latency_trace!(
                component = $component,
                operation = $operation,
                elapsed_ms,
                outcome = "ok",
                $($fields)*
            );
        }
    };
}

#[macro_export]
macro_rules! live_latency_trace_error {
    ($component:expr, $operation:expr, $started_at:expr, $error_kind:expr, $($fields:tt)*) => {
        if let Some(started_at) = $started_at {
            let elapsed_ms = $crate::elapsed_ms(started_at);
            $crate::live_latency_trace!(
                component = $component,
                operation = $operation,
                elapsed_ms,
                outcome = "error",
                error_kind = $error_kind,
                $($fields)*
            );
        }
    };
}
