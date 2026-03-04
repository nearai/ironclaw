//! Shared exponential backoff with jitter.
//!
//! Used by both the tool-level retry system (`tools::retry`) and the LLM-level
//! retry decorator (`llm::retry`). Centralised here so the formula, jitter
//! strategy, and 100ms floor are defined in exactly one place.

use std::time::Duration;

use rand::Rng;

/// Calculate an exponential backoff delay with 25% jitter.
///
/// Formula: `base_ms * 2^attempt`, optionally capped at `max_ms`, then
/// uniform jitter in `[-25%, +25%]` of the (capped) value. A hard floor
/// of 100ms prevents degenerate tight-loop retries regardless of inputs.
///
/// # Arguments
///
/// * `base_ms`  – base delay in milliseconds (e.g. 1000 for 1s)
/// * `attempt`  – zero-based attempt index (0 = first retry)
/// * `max_ms`   – optional ceiling; `None` means no cap
pub(crate) fn exponential_backoff(base_ms: u64, attempt: u32, max_ms: Option<u64>) -> Duration {
    let exp_ms = base_ms.saturating_mul(2u64.saturating_pow(attempt));
    let capped_ms = max_ms.map_or(exp_ms, |m| exp_ms.min(m));

    // 25% jitter band
    let jitter_range = capped_ms / 4;
    let jitter = if jitter_range > 0 {
        let offset = rand::thread_rng().gen_range(0..=jitter_range.saturating_mul(2));
        offset as i64 - jitter_range as i64
    } else {
        0
    };

    let delay_ms = (capped_ms as i64 + jitter).max(100) as u64;
    Duration::from_millis(delay_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_growth_with_cap() {
        for _ in 0..20 {
            // attempt 0: base 1000ms, jitter ±250 → [750, 1250]
            let d0 = exponential_backoff(1000, 0, None);
            assert!(d0.as_millis() >= 750, "attempt 0 too low: {d0:?}");
            assert!(d0.as_millis() <= 1250, "attempt 0 too high: {d0:?}");

            // attempt 1: base 2000ms, jitter ±500 → [1500, 2500]
            let d1 = exponential_backoff(1000, 1, None);
            assert!(d1.as_millis() >= 1500, "attempt 1 too low: {d1:?}");
            assert!(d1.as_millis() <= 2500, "attempt 1 too high: {d1:?}");

            // attempt 2 with 3000ms cap: jitter ±750 → [2250, 3750]
            let d2 = exponential_backoff(1000, 2, Some(3000));
            assert!(d2.as_millis() >= 2250, "capped attempt 2 too low: {d2:?}");
            assert!(d2.as_millis() <= 3750, "capped attempt 2 too high: {d2:?}");
        }
    }

    #[test]
    fn test_floor_at_100ms() {
        // Even a tiny base + cap should never go below 100ms
        for _ in 0..20 {
            let d = exponential_backoff(1, 0, Some(1));
            assert!(d.as_millis() >= 100, "below 100ms floor: {d:?}");
        }
    }

    #[test]
    fn test_no_overflow_at_high_attempts() {
        let d = exponential_backoff(1000, 40, None);
        assert!(d.as_millis() >= 100, "overflow produced sub-100ms: {d:?}");
    }
}
