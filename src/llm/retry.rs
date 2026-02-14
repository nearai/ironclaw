//! Shared retry helpers for LLM providers.
//!
//! Provides exponential backoff with jitter and retryable status classification
//! used by both `NearAiProvider` and `NearAiChatProvider`.

use std::time::Duration;

use rand::Rng;

/// Returns `true` if the HTTP status code is transient and worth retrying.
pub(crate) fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// Calculate exponential backoff delay with random jitter.
///
/// Base delay is 1 second, doubled each attempt, with +/-25% jitter.
/// - attempt 0: ~1s (0.75s - 1.25s)
/// - attempt 1: ~2s (1.5s - 2.5s)
/// - attempt 2: ~4s (3.0s - 5.0s)
pub(crate) fn retry_backoff_delay(attempt: u32) -> Duration {
    let base_ms: u64 = 1000u64.saturating_mul(2u64.saturating_pow(attempt));
    let jitter_range = base_ms / 4; // 25%
    let jitter = if jitter_range > 0 {
        let offset = rand::thread_rng().gen_range(0..=jitter_range * 2);
        offset as i64 - jitter_range as i64
    } else {
        0
    };
    let delay_ms = (base_ms as i64 + jitter).max(100) as u64;
    Duration::from_millis(delay_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_retryable_status() {
        // Transient errors should be retryable
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(502));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(504));

        // Client errors should not be retryable
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(403));
        assert!(!is_retryable_status(404));
        assert!(!is_retryable_status(422));

        // Success codes should not be retryable
        assert!(!is_retryable_status(200));
        assert!(!is_retryable_status(201));
    }

    #[test]
    fn test_retry_backoff_delay_exponential_growth() {
        // Run multiple samples to verify the range, accounting for jitter
        for _ in 0..20 {
            let d0 = retry_backoff_delay(0);
            let d1 = retry_backoff_delay(1);
            let d2 = retry_backoff_delay(2);

            // Attempt 0: base 1000ms, jitter +/-250ms -> [750, 1250]
            assert!(d0.as_millis() >= 750, "attempt 0 too low: {:?}", d0);
            assert!(d0.as_millis() <= 1250, "attempt 0 too high: {:?}", d0);

            // Attempt 1: base 2000ms, jitter +/-500ms -> [1500, 2500]
            assert!(d1.as_millis() >= 1500, "attempt 1 too low: {:?}", d1);
            assert!(d1.as_millis() <= 2500, "attempt 1 too high: {:?}", d1);

            // Attempt 2: base 4000ms, jitter +/-1000ms -> [3000, 5000]
            assert!(d2.as_millis() >= 3000, "attempt 2 too low: {:?}", d2);
            assert!(d2.as_millis() <= 5000, "attempt 2 too high: {:?}", d2);
        }
    }

    #[test]
    fn test_retry_backoff_delay_minimum() {
        // Even at attempt 0, delay should be at least 100ms (the minimum floor)
        for _ in 0..20 {
            let delay = retry_backoff_delay(0);
            assert!(delay.as_millis() >= 100);
        }
    }

    #[test]
    fn test_retry_backoff_delay_no_overflow() {
        // Very high attempt numbers should not panic from overflow
        let delay = retry_backoff_delay(30);
        assert!(delay.as_millis() >= 100);
    }
}
