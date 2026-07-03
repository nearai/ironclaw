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

#[inline]
pub fn json_value_bytes(value: &serde_json::Value) -> u64 {
    let mut counter = JsonByteCounter::default();
    serde_json::to_writer(&mut counter, value)
        .map(|()| counter.bytes)
        .unwrap_or(0)
}

#[derive(Default)]
struct JsonByteCounter {
    bytes: u64,
}

impl std::io::Write for JsonByteCounter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.bytes = self.bytes.saturating_add(buffer.len() as u64);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
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
    use std::io::Write as _;

    use serde_json::json;

    use super::*;

    #[test]
    fn json_value_bytes_matches_serialized_value_length() {
        let value = json!({
            "message": "hello",
            "count": 3,
            "items": ["a", "b"]
        });

        assert_eq!(
            json_value_bytes(&value),
            serde_json::to_vec(&value).unwrap().len() as u64
        );
    }

    #[test]
    fn json_byte_counter_saturates_on_write() {
        let mut counter = JsonByteCounter {
            bytes: u64::MAX - 1,
        };

        counter.write_all(b"abc").unwrap();

        assert_eq!(counter.bytes, u64::MAX);
    }
}
