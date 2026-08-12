//! Zero-cost-when-off latency-trace macros over the `ironclaw_latency` target.
//!
//! Everything here is either a macro or a helper the macros need. The crate
//! holds exactly one dependency, `tracing`, and that is the charter: a crate
//! that wants to time an operation can take this without acquiring anything
//! else. Nothing that merely *produces a value a trace happens to record*
//! belongs here — that measurement belongs to whoever produces the thing being
//! measured (see `crates/substrates/ironclaw_observability/AGENTS.md`).
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn elapsed_ms_saturates_instead_of_wrapping() {
        // The only arithmetic in the crate. `u128 -> u64` can overflow for an
        // absurd interval; it must clamp rather than wrap, because a wrapped
        // duration reads as a *fast* operation in a latency trace.
        assert_eq!(elapsed_ms(Instant::now()), 0);
        let long_ago = Instant::now()
            .checked_sub(Duration::from_millis(1_500))
            .expect("1.5s before now is representable");
        assert!(elapsed_ms(long_ago) >= 1_500);
    }

    #[test]
    fn started_at_is_none_when_the_latency_target_is_off() {
        // No subscriber is installed in this test binary, so the
        // `ironclaw_latency` TRACE target is disabled — the zero-cost-when-off
        // property the whole crate exists for.
        assert!(!live_latency_enabled());
        assert!(live_latency_started_at().is_none());
    }
}
