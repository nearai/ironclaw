//! Pluggable backend for predicate sliding-window state.
//!
//! The [`PredicateEvaluator`] in [`crate::evaluator`] delegates its
//! counter / value-sum bookkeeping to a [`PredicateStateBackend`]. The
//! default backend ([`InMemoryPredicateStateBackend`]) preserves the
//! existing in-process semantics; future durable backends
//! (Postgres-backed, libSQL-backed) implement the same trait so the
//! evaluator's predicate logic stays backend-agnostic.
//!
//! # Why a trait
//!
//! - **Cross-process consistency**: Hook 1 increments → Hook 2 (different
//!   process, same tenant) reads the updated count.
//! - **Restart survival**: counter state survives process restart.
//! - **Replay refusal**: each record call carries a [`PredicateEventId`].
//!   Re-emitting a recorded `event_id` is a no-op against the count —
//!   the in-memory backend skips, durable backends `INSERT … ON CONFLICT
//!   DO NOTHING`. This is the load-bearing property for replay safety
//!   (codex Critical on PR #3635).
//!
//! # Atomic record-and-read
//!
//! Each [`record_invocation`] / [`record_value`] call performs the
//! write AND returns the resulting in-window count/sum. The two
//! operations must be atomic against concurrent writers (codex
//! Critical on PR #3635) — splitting them into `record` + `read`
//! would let two hosts each see "1 under cap" and both proceed,
//! letting the cap drift past `max`. Implementations must hold a
//! single lock / transaction across the write + read.
//!
//! # Sync trait, monotonic clock
//!
//! The trait is **synchronous** in this PR to minimize call-site churn.
//! [`Instant`] is monotonic (process-local) and is the right clock for
//! the in-memory backend; durable backends will accept a
//! [`std::time::SystemTime`]-based companion via a separate trait or
//! adapter (tracked in the scope doc) since `Instant` cannot be
//! serialized across processes.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};

use ironclaw_host_api::TenantId;
use rust_decimal::Decimal;

use crate::identity::HookId;

/// Identity of an invocation-history bucket. The `tenant_id` field is
/// the trust boundary — one tenant's counter must never affect another's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InvocationKey {
    pub hook_id: HookId,
    pub tenant_id: TenantId,
    pub capability: String,
}

/// Identity of a value-sum-history bucket. Extends [`InvocationKey`]
/// with the numeric field path the predicate is summing over.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValueKey {
    pub hook_id: HookId,
    pub tenant_id: TenantId,
    pub capability: String,
    pub field: String,
}

/// Backend-agnostic identity for a recorded predicate event. Backends
/// dedupe on this id so a re-emitted invocation is counted exactly
/// once across replay / retries. The id is opaque to the backend —
/// callers can stamp it with whatever uniquely identifies the
/// originating event (e.g. a `RuntimeEventId` hex, an arguments digest
/// + timestamp tuple, a synthetic UUID for tests).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PredicateEventId(pub String);

/// Error surface for the backend. Durable backends use this to signal
/// transient failures (connection lost, DB unavailable); the in-memory
/// backend never returns errors.
#[derive(Debug, thiserror::Error)]
pub enum PredicateBackendError {
    #[error("predicate state backend is unavailable: {0}")]
    Unavailable(String),
}

/// Backend for the predicate evaluator's sliding-window state.
///
/// Each method is bracketed by `now` so tests can drive a deterministic
/// clock; production callers pass [`Instant::now()`].
///
/// # Atomicity contract
///
/// Each `record_*` call **must** perform its write AND its read of the
/// resulting in-window count/sum atomically against concurrent writers.
/// Implementations holding the same lock / transaction across both
/// halves are correct; splitting them is a race that lets the cap
/// drift past `max` (codex Critical on PR #3635).
///
/// # Replay refusal contract
///
/// When `event_id` matches a previously-recorded event for the same
/// `key`, the call is a no-op against the stored history. The backend
/// still returns the current in-window count/sum, but the duplicate
/// `event_id` does not contribute a second entry. The in-memory
/// backend implements this via a small per-key seen-id set; durable
/// backends should use `INSERT … ON CONFLICT (event_id) DO NOTHING`
/// or equivalent.
pub trait PredicateStateBackend: Send + Sync {
    /// Record an invocation at `now` against `key` (idempotent against
    /// `event_id`) and return the resulting in-window count after
    /// trimming entries older than `window`. Atomic against concurrent
    /// writers.
    fn record_invocation(
        &self,
        key: &InvocationKey,
        event_id: &PredicateEventId,
        now: Instant,
        window: Duration,
    ) -> Result<u32, PredicateBackendError>;

    /// Record a numeric value at `now` against `key` (idempotent
    /// against `event_id`) and return the resulting in-window sum
    /// after trimming. Atomic against concurrent writers.
    fn record_value(
        &self,
        key: &ValueKey,
        event_id: &PredicateEventId,
        now: Instant,
        value: Decimal,
        window: Duration,
    ) -> Result<Decimal, PredicateBackendError>;

    /// Total LRU evictions observed since construction. Operators
    /// should alert when this counter advances. Threat-model finding D5.
    fn evictions_observed(&self) -> u64;
}

/// In-process backend. Preserves the original [`PredicateEvaluator`]
/// semantics: tenant-keyed sliding windows, LRU eviction at
/// [`MAX_HISTORY_KEYS`] per map, and the `evictions` counter for
/// operator monitoring.
///
/// State is per-instance; cross-process consistency + restart survival
/// require a durable backend (separate PR).
/// Per-key entry tracking both the timestamp ring and a small bounded
/// set of recently-seen `event_id`s for replay dedup. Implementations
/// keep at most `RECENT_EVENT_ID_CAP` ids per key; older ones age out
/// with the timestamp ring (a re-emitted event much older than the
/// window is treated as a fresh record, which is the right behavior
/// for windowed predicates).
#[derive(Debug, Default)]
struct InvocationBucket {
    entries: VecDeque<Instant>,
    recent_ids: VecDeque<PredicateEventId>,
}

#[derive(Debug, Default)]
struct ValueBucket {
    entries: VecDeque<(Instant, Decimal)>,
    recent_ids: VecDeque<PredicateEventId>,
}

/// Cap on recently-seen event-id memory per key. Bigger than typical
/// in-window event volume, small enough that LRU pressure on the
/// outer history map dominates the memory budget.
const RECENT_EVENT_ID_CAP: usize = 256;

pub struct InMemoryPredicateStateBackend {
    invocation_history: Mutex<HashMap<InvocationKey, InvocationBucket>>,
    value_history: Mutex<HashMap<ValueKey, ValueBucket>>,
    evictions: AtomicU64,
}

/// Maximum number of distinct keys retained per history map. Bounds the
/// in-memory backend's memory footprint against threat-model finding **D5**.
pub const MAX_HISTORY_KEYS: usize = 8_192;

impl InMemoryPredicateStateBackend {
    pub fn new() -> Self {
        Self {
            invocation_history: Mutex::new(HashMap::new()),
            value_history: Mutex::new(HashMap::new()),
            evictions: AtomicU64::new(0),
        }
    }
}

impl Default for InMemoryPredicateStateBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl PredicateStateBackend for InMemoryPredicateStateBackend {
    fn record_invocation(
        &self,
        key: &InvocationKey,
        event_id: &PredicateEventId,
        now: Instant,
        window: Duration,
    ) -> Result<u32, PredicateBackendError> {
        // Single-lock atomic record-and-read (codex Critical: must not
        // split write + read across concurrent writers).
        let mut history = self
            .invocation_history
            .lock()
            .expect("predicate history mutex poisoned"); // safety: mutex poison means another thread panicked; failing closed here is correct
        if !history.contains_key(key) && history.len() >= MAX_HISTORY_KEYS {
            evict_lru_invocation(&mut history, &self.evictions);
        }
        let bucket = history.entry(key.clone()).or_default();
        let cutoff = now.checked_sub(window).unwrap_or(now);
        while let Some(front) = bucket.entries.front() {
            if *front < cutoff {
                bucket.entries.pop_front();
            } else {
                break;
            }
        }
        let duplicate = bucket.recent_ids.contains(event_id);
        if !duplicate {
            bucket.entries.push_back(now);
            bucket.recent_ids.push_back(event_id.clone());
            while bucket.recent_ids.len() > RECENT_EVENT_ID_CAP {
                bucket.recent_ids.pop_front();
            }
        }
        Ok(bucket.entries.len() as u32)
    }

    fn record_value(
        &self,
        key: &ValueKey,
        event_id: &PredicateEventId,
        now: Instant,
        value: Decimal,
        window: Duration,
    ) -> Result<Decimal, PredicateBackendError> {
        let mut history = self
            .value_history
            .lock()
            .expect("predicate value history mutex poisoned"); // safety: mutex poison means another thread panicked; failing closed here is correct
        if !history.contains_key(key) && history.len() >= MAX_HISTORY_KEYS {
            evict_lru_value(&mut history, &self.evictions);
        }
        let bucket = history.entry(key.clone()).or_default();
        let cutoff = now.checked_sub(window).unwrap_or(now);
        while let Some((ts, _)) = bucket.entries.front() {
            if *ts < cutoff {
                bucket.entries.pop_front();
            } else {
                break;
            }
        }
        let duplicate = bucket.recent_ids.contains(event_id);
        if !duplicate {
            bucket.entries.push_back((now, value));
            bucket.recent_ids.push_back(event_id.clone());
            while bucket.recent_ids.len() > RECENT_EVENT_ID_CAP {
                bucket.recent_ids.pop_front();
            }
        }
        Ok(bucket.entries.iter().map(|(_, v)| *v).sum())
    }

    fn evictions_observed(&self) -> u64 {
        self.evictions.load(AtomicOrdering::Relaxed)
    }
}

/// Evict the entry with the earliest "front" timestamp — that is, the key
/// whose oldest retained sample is older than any other key's oldest
/// sample. Conservative LRU approximation.
fn evict_lru_invocation(
    history: &mut HashMap<InvocationKey, InvocationBucket>,
    evictions: &AtomicU64,
) {
    let victim = history
        .iter()
        .filter_map(|(k, v)| v.entries.front().map(|ts| (k.clone(), *ts)))
        .min_by_key(|(_, ts)| *ts)
        .map(|(k, _)| k);
    if let Some(k) = victim {
        history.remove(&k);
        evictions.fetch_add(1, AtomicOrdering::Relaxed);
    }
}

fn evict_lru_value(history: &mut HashMap<ValueKey, ValueBucket>, evictions: &AtomicU64) {
    let victim = history
        .iter()
        .filter_map(|(k, v)| v.entries.front().map(|(ts, _)| (k.clone(), *ts)))
        .min_by_key(|(_, ts)| *ts)
        .map(|(k, _)| k);
    if let Some(k) = victim {
        history.remove(&k);
        evictions.fetch_add(1, AtomicOrdering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ExtensionId, HookLocalId, HookVersion};
    use rust_decimal::Decimal;

    fn hook_id() -> HookId {
        HookId::derive(
            &ExtensionId("ext".to_string()),
            "1.0",
            &HookLocalId("h".to_string()),
            HookVersion::ONE,
        )
    }

    fn tenant() -> TenantId {
        TenantId::new("alpha").expect("ok")
    }

    fn ev(s: &str) -> PredicateEventId {
        PredicateEventId(s.to_string())
    }

    #[test]
    fn in_memory_invocation_counts_within_window() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.x".to_string(),
        };
        let now = Instant::now();
        let count_1 = backend
            .record_invocation(&key, &ev("e1"), now, Duration::from_secs(60))
            .expect("ok");
        let count_2 = backend
            .record_invocation(
                &key,
                &ev("e2"),
                now + Duration::from_secs(1),
                Duration::from_secs(60),
            )
            .expect("ok");
        assert_eq!(count_1, 1);
        assert_eq!(count_2, 2);
    }

    #[test]
    fn in_memory_invocation_trims_outside_window() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.x".to_string(),
        };
        let t0 = Instant::now();
        let _ = backend
            .record_invocation(&key, &ev("e1"), t0, Duration::from_secs(10))
            .expect("ok");
        let count = backend
            .record_invocation(
                &key,
                &ev("e2"),
                t0 + Duration::from_secs(100),
                Duration::from_secs(10),
            )
            .expect("ok");
        assert_eq!(count, 1, "earlier entry should be trimmed");
    }

    #[test]
    fn in_memory_value_sums_within_window() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = ValueKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.x".to_string(),
            field: "amount".to_string(),
        };
        let now = Instant::now();
        let sum_1 = backend
            .record_value(
                &key,
                &ev("e1"),
                now,
                Decimal::from(50),
                Duration::from_secs(60),
            )
            .expect("ok");
        let sum_2 = backend
            .record_value(
                &key,
                &ev("e2"),
                now + Duration::from_secs(1),
                Decimal::from(75),
                Duration::from_secs(60),
            )
            .expect("ok");
        assert_eq!(sum_1, Decimal::from(50));
        assert_eq!(sum_2, Decimal::from(125));
    }

    #[test]
    fn in_memory_tenant_isolation() {
        let backend = InMemoryPredicateStateBackend::new();
        let alpha_key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: TenantId::new("alpha").expect("ok"),
            capability: "cap.x".to_string(),
        };
        let beta_key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: TenantId::new("beta").expect("ok"),
            capability: "cap.x".to_string(),
        };
        let now = Instant::now();
        backend
            .record_invocation(&alpha_key, &ev("e1"), now, Duration::from_secs(60))
            .expect("ok");
        backend
            .record_invocation(&alpha_key, &ev("e2"), now, Duration::from_secs(60))
            .expect("ok");
        backend
            .record_invocation(&alpha_key, &ev("e3"), now, Duration::from_secs(60))
            .expect("ok");
        let beta_count = backend
            .record_invocation(&beta_key, &ev("b1"), now, Duration::from_secs(60))
            .expect("ok");
        assert_eq!(
            beta_count, 1,
            "tenant β's counter must not inherit α's increments"
        );
    }

    /// Replay refusal: re-emitting the same `event_id` against an
    /// already-recorded invocation must NOT increment the count.
    /// Load-bearing for safe replay across host restarts (codex
    /// Critical on PR #3635).
    #[test]
    fn in_memory_duplicate_event_id_is_a_noop_for_invocations() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.x".to_string(),
        };
        let now = Instant::now();
        let count_1 = backend
            .record_invocation(&key, &ev("event-A"), now, Duration::from_secs(60))
            .expect("ok");
        let count_2 = backend
            .record_invocation(
                &key,
                &ev("event-A"),
                now + Duration::from_secs(1),
                Duration::from_secs(60),
            )
            .expect("ok");
        let count_3 = backend
            .record_invocation(
                &key,
                &ev("event-B"),
                now + Duration::from_secs(2),
                Duration::from_secs(60),
            )
            .expect("ok");
        assert_eq!(count_1, 1);
        assert_eq!(count_2, 1, "duplicate event-A must not increment");
        assert_eq!(count_3, 2, "distinct event-B advances the count");
    }

    #[test]
    fn in_memory_duplicate_event_id_is_a_noop_for_values() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = ValueKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.x".to_string(),
            field: "amount".to_string(),
        };
        let now = Instant::now();
        let sum_1 = backend
            .record_value(
                &key,
                &ev("event-A"),
                now,
                Decimal::from(50),
                Duration::from_secs(60),
            )
            .expect("ok");
        let sum_2 = backend
            .record_value(
                &key,
                &ev("event-A"),
                now + Duration::from_secs(1),
                Decimal::from(50),
                Duration::from_secs(60),
            )
            .expect("ok");
        let sum_3 = backend
            .record_value(
                &key,
                &ev("event-B"),
                now + Duration::from_secs(2),
                Decimal::from(75),
                Duration::from_secs(60),
            )
            .expect("ok");
        assert_eq!(sum_1, Decimal::from(50));
        assert_eq!(
            sum_2,
            Decimal::from(50),
            "duplicate event-A must not double-count the value"
        );
        assert_eq!(sum_3, Decimal::from(125));
    }
}
