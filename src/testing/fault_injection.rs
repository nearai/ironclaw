//! Fault injection framework for testing retry, failover, and circuit breaker behavior.
//!
//! Provides [`FaultInjector`] which can be attached to [`StubLlm`](super::StubLlm) to
//! produce configurable error sequences, random failures, and delays.
//!
//! # Example
//!
//! ```rust,no_run
//! use ironclaw::testing::fault_injection::*;
//!
//! // Fail twice with transient errors, then succeed
//! let injector = FaultInjector::sequence([
//!     FaultAction::Fail(FaultType::RequestFailed),
//!     FaultAction::Fail(FaultType::RateLimited { retry_after: None }),
//!     FaultAction::Succeed,
//! ]);
//! ```

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use crate::llm::error::LlmError;

/// The type of fault to inject.
#[derive(Debug, Clone)]
pub enum FaultType {
    /// Transient request failure (retryable).
    RequestFailed,
    /// Rate limited with optional retry-after duration.
    RateLimited { retry_after: Option<Duration> },
    /// Authentication failure (non-retryable).
    AuthFailed,
    /// Invalid response from provider (retryable).
    InvalidResponse,
    /// I/O error (retryable).
    IoError,
    /// Context length exceeded (non-retryable).
    ContextLengthExceeded,
    /// Session expired (transient for circuit breaker, not retryable).
    SessionExpired,
}

impl FaultType {
    /// Convert to the corresponding `LlmError`.
    pub fn to_llm_error(&self, provider: &str) -> LlmError {
        match self {
            FaultType::RequestFailed => LlmError::RequestFailed {
                provider: provider.to_string(),
                reason: "injected fault: request failed".to_string(),
            },
            FaultType::RateLimited { retry_after } => LlmError::RateLimited {
                provider: provider.to_string(),
                retry_after: *retry_after,
            },
            FaultType::AuthFailed => LlmError::AuthFailed {
                provider: provider.to_string(),
            },
            FaultType::InvalidResponse => LlmError::InvalidResponse {
                provider: provider.to_string(),
                reason: "injected fault: invalid response".to_string(),
            },
            FaultType::IoError => LlmError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "injected fault: connection reset",
            )),
            FaultType::ContextLengthExceeded => LlmError::ContextLengthExceeded {
                used: 100_000,
                limit: 50_000,
            },
            FaultType::SessionExpired => LlmError::SessionExpired {
                provider: provider.to_string(),
            },
        }
    }
}

/// Action to take on a given call.
#[derive(Debug, Clone)]
pub enum FaultAction {
    /// Return a successful response.
    Succeed,
    /// Return an error of the given type.
    Fail(FaultType),
    /// Sleep for the given duration, then succeed.
    Delay(Duration),
}

/// How the fault sequence is consumed.
#[derive(Debug, Clone)]
pub enum FaultMode {
    /// Play the sequence once, then succeed for all subsequent calls.
    SequenceOnce,
    /// Loop the sequence forever.
    SequenceLoop,
    /// Fail randomly at the given rate (0.0 = never, 1.0 = always) with
    /// the specified fault type. Uses a seeded RNG for reproducibility.
    Random { error_rate: f64, fault: FaultType },
}

/// A configurable fault injector for [`StubLlm`](super::StubLlm).
///
/// Thread-safe: uses atomic call counter and mutex-protected RNG.
pub struct FaultInjector {
    actions: Vec<FaultAction>,
    mode: FaultMode,
    call_index: AtomicU32,
    /// Seeded RNG for Random mode, behind Mutex for Sync.
    rng_state: Mutex<u64>,
}

impl std::fmt::Debug for FaultInjector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FaultInjector")
            .field("call_index", &self.call_index.load(Ordering::Relaxed))
            .field("mode", &self.mode)
            .finish()
    }
}

impl FaultInjector {
    /// Create a fault injector that plays actions once, then succeeds.
    pub fn sequence(actions: impl IntoIterator<Item = FaultAction>) -> Self {
        Self {
            actions: actions.into_iter().collect(),
            mode: FaultMode::SequenceOnce,
            call_index: AtomicU32::new(0),
            rng_state: Mutex::new(0),
        }
    }

    /// Create a fault injector that loops the action sequence forever.
    pub fn sequence_loop(actions: impl IntoIterator<Item = FaultAction>) -> Self {
        Self {
            actions: actions.into_iter().collect(),
            mode: FaultMode::SequenceLoop,
            call_index: AtomicU32::new(0),
            rng_state: Mutex::new(0),
        }
    }

    /// Create a fault injector with random failures at the given rate.
    ///
    /// The seed is guarded against zero, which is a fixed point for xorshift.
    pub fn random(error_rate: f64, fault: FaultType, seed: u64) -> Self {
        let seed = if seed == 0 { 1 } else { seed };
        Self {
            actions: Vec::new(),
            mode: FaultMode::Random { error_rate, fault },
            call_index: AtomicU32::new(0),
            rng_state: Mutex::new(seed),
        }
    }

    /// Get the action for the next call.
    pub fn next_action(&self) -> FaultAction {
        let index = self.call_index.fetch_add(1, Ordering::Relaxed) as usize;

        match &self.mode {
            FaultMode::SequenceOnce => {
                if index < self.actions.len() {
                    self.actions[index].clone()
                } else {
                    FaultAction::Succeed
                }
            }
            FaultMode::SequenceLoop => {
                if self.actions.is_empty() {
                    FaultAction::Succeed
                } else {
                    self.actions[index % self.actions.len()].clone()
                }
            }
            FaultMode::Random { error_rate, fault } => {
                // Simple xorshift64 PRNG for reproducible randomness.
                let random_val = {
                    let mut state = self.rng_state.lock().unwrap_or_else(|p| p.into_inner());
                    *state ^= *state << 13;
                    *state ^= *state >> 7;
                    *state ^= *state << 17;
                    (*state as f64) / (u64::MAX as f64)
                };
                if random_val < *error_rate {
                    FaultAction::Fail(fault.clone())
                } else {
                    FaultAction::Succeed
                }
            }
        }
    }

    /// Get the total number of calls made.
    pub fn call_count(&self) -> u32 {
        self.call_index.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_once_plays_then_succeeds() {
        let injector = FaultInjector::sequence([
            FaultAction::Fail(FaultType::RequestFailed),
            FaultAction::Fail(FaultType::RateLimited { retry_after: None }),
            FaultAction::Succeed,
        ]);

        // First two calls should fail
        assert!(matches!(
            injector.next_action(),
            FaultAction::Fail(FaultType::RequestFailed)
        ));
        assert!(matches!(
            injector.next_action(),
            FaultAction::Fail(FaultType::RateLimited { .. })
        ));
        // Third call is explicit succeed
        assert!(matches!(injector.next_action(), FaultAction::Succeed));
        // Beyond sequence: implicit succeed
        assert!(matches!(injector.next_action(), FaultAction::Succeed));
        assert!(matches!(injector.next_action(), FaultAction::Succeed));
        assert_eq!(injector.call_count(), 5);
    }

    #[test]
    fn sequence_loop_repeats() {
        let injector = FaultInjector::sequence_loop([
            FaultAction::Fail(FaultType::RequestFailed),
            FaultAction::Succeed,
        ]);

        assert!(matches!(injector.next_action(), FaultAction::Fail(_)));
        assert!(matches!(injector.next_action(), FaultAction::Succeed));
        assert!(matches!(injector.next_action(), FaultAction::Fail(_)));
        assert!(matches!(injector.next_action(), FaultAction::Succeed));
    }

    #[test]
    fn random_mode_is_deterministic_with_seed() {
        let injector1 = FaultInjector::random(0.5, FaultType::RequestFailed, 42);
        let injector2 = FaultInjector::random(0.5, FaultType::RequestFailed, 42);

        let results1: Vec<bool> = (0..20)
            .map(|_| matches!(injector1.next_action(), FaultAction::Fail(_)))
            .collect();
        let results2: Vec<bool> = (0..20)
            .map(|_| matches!(injector2.next_action(), FaultAction::Fail(_)))
            .collect();

        assert_eq!(results1, results2, "Same seed should produce same sequence");
    }

    #[test]
    fn fault_type_produces_correct_llm_errors() {
        let provider = "test-provider";

        assert!(matches!(
            FaultType::RequestFailed.to_llm_error(provider),
            LlmError::RequestFailed { .. }
        ));
        assert!(matches!(
            FaultType::RateLimited {
                retry_after: Some(Duration::from_secs(5))
            }
            .to_llm_error(provider),
            LlmError::RateLimited { .. }
        ));
        assert!(matches!(
            FaultType::AuthFailed.to_llm_error(provider),
            LlmError::AuthFailed { .. }
        ));
        assert!(matches!(
            FaultType::InvalidResponse.to_llm_error(provider),
            LlmError::InvalidResponse { .. }
        ));
        assert!(matches!(
            FaultType::IoError.to_llm_error(provider),
            LlmError::Io(_)
        ));
        assert!(matches!(
            FaultType::ContextLengthExceeded.to_llm_error(provider),
            LlmError::ContextLengthExceeded { .. }
        ));
        assert!(matches!(
            FaultType::SessionExpired.to_llm_error(provider),
            LlmError::SessionExpired { .. }
        ));
    }

    #[test]
    fn delay_action_exists() {
        let injector = FaultInjector::sequence([FaultAction::Delay(Duration::from_millis(100))]);
        assert!(matches!(injector.next_action(), FaultAction::Delay(_)));
    }

    #[test]
    fn random_seed_zero_does_not_always_fail() {
        // seed=0 is a fixed point for xorshift; the constructor guards it to 1.
        let injector = FaultInjector::random(0.5, FaultType::RequestFailed, 0);
        let results: Vec<bool> = (0..20)
            .map(|_| matches!(injector.next_action(), FaultAction::Fail(_)))
            .collect();
        // With a working RNG, not every call should fail at 50% error rate.
        let all_same = results.iter().all(|&r| r == results[0]);
        assert!(!all_same, "seed=0 should not produce degenerate output");
    }

    #[test]
    fn empty_sequence_always_succeeds() {
        let injector = FaultInjector::sequence([]);
        for _ in 0..10 {
            assert!(matches!(injector.next_action(), FaultAction::Succeed));
        }
    }
}
