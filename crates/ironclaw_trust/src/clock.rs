//! Pluggable time source for trust-policy evaluation.
//!
//! The default [`SystemClock`] reads `chrono::Utc::now()`, which is what
//! production deployments want for accurate audit timestamps. Tests can
//! inject a deterministic clock via [`HostTrustPolicy::with_clock`]
//! (defined in `crate::policy`) so `evaluated_at` is reproducible across
//! runs.

use chrono::{DateTime, Utc};
use ironclaw_host_api::Timestamp;

/// Time source consumed by `HostTrustPolicy`.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// Default production clock — reads the system wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Utc::now()
    }
}

/// Test fixture: a clock that returns a fixed timestamp on every call.
/// Defined unconditionally because it is not a privilege-construction
/// surface — fabricating a timestamp grants no authority. Useful in any
/// context where deterministic evaluation is needed (audit golden files,
/// replay harnesses, fuzzers).
#[derive(Debug, Clone, Copy)]
pub struct FixedClock {
    pub instant: DateTime<Utc>,
}

impl FixedClock {
    pub fn new(instant: DateTime<Utc>) -> Self {
        Self { instant }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.instant
    }
}
