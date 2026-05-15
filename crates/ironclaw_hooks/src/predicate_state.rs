//! Pluggable backend for predicate sliding-window state.
//!
//! The [`PredicateEvaluator`] in [`crate::evaluator`] delegates its
//! counter / value-sum bookkeeping to a [`PredicateStateBackend`]. The
//! default backend ([`InMemoryPredicateStateBackend`]) preserves the
//! existing in-process semantics; future durable backends
//! (Postgres-backed, libSQL-backed) implement the same trait so the
//! evaluator's predicate logic stays backend-agnostic.
//!
//! # Visibility
//!
//! The backend trait and the in-memory impl are intentionally
//! `pub(crate)` until the durable contract is stable. The trait's `now:
//! Instant` is correct for the in-memory backend but not serializable
//! across processes, so a future durable trait will accept
//! `chrono::DateTime<Utc>` (see successor doc 03-persistent-counter.md).
//! Holding the public surface back until then avoids shipping a public
//! API we know we'll break (serrrfirat MED on PR #3635).
//!
//! [`PredicateEventId`] stays `pub` because it flows through
//! [`BeforeCapabilityHookContext::caller_event_id`] on the public hook
//! surface; only the backend trait and its keys/error type are crate-
//! internal until the durable contract is ready.
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

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};

use ironclaw_host_api::TenantId;
use rust_decimal::Decimal;

use crate::identity::HookId;

/// Identity of an invocation-history bucket. The `tenant_id` field is
/// the trust boundary — one tenant's counter must never affect another's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct InvocationKey {
    pub hook_id: HookId,
    pub tenant_id: TenantId,
    pub capability: String,
}

/// Identity of a value-sum-history bucket. Extends [`InvocationKey`]
/// with the numeric field path the predicate is summing over.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ValueKey {
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
///
/// # Trust boundary — host-assigned, NEVER tenant-supplied
///
/// `PredicateEventId` is a **host-assigned** identity. The host runtime
/// is responsible for minting it from authoritative sources it controls
/// (the dispatcher's `RuntimeEventId`, an arguments digest, a host-side
/// blake3 hash) before threading it through
/// [`BeforeCapabilityHookContext::caller_event_id`].
///
/// It MUST NOT be propagated unchanged from any tenant-controlled
/// surface (capability arguments, manifest fields, extension WASM
/// memory, untrusted HTTP request bodies). A tenant that can choose
/// its own `PredicateEventId` can either:
///
/// - **Undercount themselves into infinity** by sending the same id on
///   every call — the backend treats every invocation as a duplicate
///   and the rate cap never fires.
/// - **Poison another tenant's bucket** if id namespaces ever overlap
///   (the per-key scoping defends against this, but the trust property
///   is "host-assigned" — don't trust it to be tenant-isolated).
///
/// The validation in [`Self::new`] (non-empty, NUL-free) is a format
/// invariant for durable backends, NOT a trust check. Empty / NUL
/// rejection alone does not make a tenant-supplied id safe.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PredicateEventId(String);

/// Format-validation error for [`PredicateEventId::new`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PredicateEventIdError {
    #[error("predicate event id must not be empty")]
    Empty,
    #[error("predicate event id must not contain NUL bytes")]
    ContainsNul,
}

impl PredicateEventId {
    /// Construct from any string-like value, validating non-empty and
    /// NUL-free at the type boundary. Returns
    /// [`PredicateEventIdError`] on rejection. Validation happens HERE
    /// (not at the `BeforeCapabilityHookContext::with_caller_event_id`
    /// setter) because the field on the context is `pub` and a caller
    /// could otherwise bypass the setter to assign an invalid id —
    /// henrypark133 MEDIUM regression from serrrfirat's 5-15 review on
    /// PR #3635.
    ///
    /// For tests and the evaluator's internal synth path that mint ids
    /// from known-good shapes (e.g. blake3 hex digests, hash-counter
    /// tuples), use [`Self::new_unchecked`] to bypass the per-call
    /// validation.
    pub fn new(value: impl Into<String>) -> Result<Self, PredicateEventIdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(PredicateEventIdError::Empty);
        }
        if value.as_bytes().contains(&0) {
            return Err(PredicateEventIdError::ContainsNul);
        }
        Ok(Self(value))
    }

    /// Construct without per-call validation. Reserved for internal
    /// synth paths and tests that mint ids from formats already known
    /// to satisfy the contract (e.g. fixed-length hex digests).
    /// External callers should use [`Self::new`].
    pub fn new_unchecked(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Error surface for the backend. Durable backends use this to signal
/// transient failures (connection lost, DB unavailable); the in-memory
/// backend never returns errors.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PredicateBackendError {
    #[error("predicate state backend is unavailable: {0}")]
    #[allow(dead_code)] // populated by future durable backends; in-memory is infallible
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
/// # Trust boundary on `event_id`
///
/// The `event_id` argument is a **host-assigned** identity, per
/// [`PredicateEventId`]'s trust-boundary docs. Backend implementations
/// MAY assume `event_id` was minted by trusted host code from
/// authoritative sources (dispatcher event id, host-side hash); they
/// MUST NOT treat it as adversarial input from the tenant. The host is
/// responsible for ensuring no tenant-controlled bytes flow into
/// `event_id` unfiltered — see [`PredicateEventId`] for the threat
/// description. The format invariants enforced by
/// [`PredicateEventId::new`] (non-empty, NUL-free) are a durability
/// contract for SQL backends, NOT a trust check.
///
/// # Replay refusal contract
///
/// Dedup is scoped to the counter `key`, NOT to `event_id` globally.
/// When `event_id` matches a previously-recorded event **for the same
/// `key`**, the call is a no-op against that key's history; the same
/// `event_id` against a *different* `key` still records normally. The
/// in-memory backend implements this via a per-key seen-id set; durable
/// backends should use `INSERT … ON CONFLICT (tenant_id, hook_id,
/// capability[, field], event_id) DO NOTHING` so two predicate-backed
/// hooks observing the same capability invocation (which share a
/// `caller_event_id`) don't undercount each other (serrrfirat HIGH on
/// PR #3635 5-15 review — see also the schema in successor doc
/// `03-persistent-counter.md`).
pub(crate) trait PredicateStateBackend: Send + Sync {
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

    /// Garbage-collect entries older than `cutoff`. Default implementation
    /// is a no-op — the in-memory backend already trims per-key on every
    /// `record_*` call, so a separate reaper would be redundant. Durable
    /// backends override this with a `DELETE WHERE occurred_at < cutoff`
    /// equivalent, run periodically by an operator reaper task.
    ///
    /// Lockable into the trait signature now (rather than at the durable
    /// backend PR) so trait-object callers don't break when the durable
    /// impl lands — henrypark133 important #5 on PR #3635.
    #[allow(dead_code)] // operator reaper hook for future durable backends
    fn evict_older_than(&self, _cutoff: Instant) -> Result<u64, PredicateBackendError> {
        Ok(0)
    }
}

/// Per-key history bucket. Maintains `entries` (the FIFO window) AND a
/// companion `dedup_ids` set so duplicate-id checks are O(1) instead of
/// O(n) — the previous linear scan serialized every evaluator call under
/// the outer mutex at high in-window entry counts (henrypark133 must-fix
/// #1 on PR #3635).
///
/// Invariants:
///
/// - `dedup_ids` is exactly the set of `event_id` values currently in
///   `entries`; every push/pop updates both.
/// - No silent dedup loss under load (codex P1 #1): the dedup memory is
///   the in-window entry set itself, not a fixed-size ring.
/// - Empty buckets get removed (codex P1 #2): when `entries` is trimmed
///   to empty, the bucket is dropped from the outer map so it can't
///   become a zombie LRU-skip.
#[derive(Debug, Default)]
struct InvocationBucket {
    entries: VecDeque<(Instant, PredicateEventId)>,
    dedup_ids: HashSet<PredicateEventId>,
}

impl InvocationBucket {
    fn pop_front(&mut self) {
        if let Some((_, id)) = self.entries.pop_front() {
            self.dedup_ids.remove(&id);
        }
    }

    fn push_back(&mut self, ts: Instant, event_id: PredicateEventId) {
        self.dedup_ids.insert(event_id.clone());
        self.entries.push_back((ts, event_id));
    }
}

#[derive(Debug, Default)]
struct ValueBucket {
    entries: VecDeque<(Instant, Decimal, PredicateEventId)>,
    dedup_ids: HashSet<PredicateEventId>,
}

impl ValueBucket {
    fn pop_front(&mut self) {
        if let Some((_, _, id)) = self.entries.pop_front() {
            self.dedup_ids.remove(&id);
        }
    }

    fn push_back(&mut self, ts: Instant, value: Decimal, event_id: PredicateEventId) {
        self.dedup_ids.insert(event_id.clone());
        self.entries.push_back((ts, value, event_id));
    }
}

/// In-process backend. Preserves the original [`PredicateEvaluator`]
/// semantics: tenant-keyed sliding windows, LRU eviction at
/// [`MAX_HISTORY_KEYS`] per map, and the `evictions` counter for
/// operator monitoring.
///
/// State is per-instance; cross-process consistency + restart survival
/// require a durable backend (separate PR).
pub(crate) struct InMemoryPredicateStateBackend {
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
        //
        // Recover from a poisoned mutex by reading the inner value rather
        // than cascading the panic to every subsequent caller
        // (henrypark133 must-fix #2 on PR #3635). A poisoning thread has
        // already aborted; refusing service indefinitely is worse than
        // proceeding with possibly-incomplete state.
        let mut history = match self.invocation_history.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !history.contains_key(key) && history.len() >= MAX_HISTORY_KEYS {
            evict_lru_invocation(&mut history, &self.evictions);
        }
        let bucket = history.entry(key.clone()).or_default();
        let cutoff = now.checked_sub(window).unwrap_or(now);
        // Trim entries outside the window via the bucket helper so the
        // `dedup_ids` set stays in sync.
        while let Some((front_ts, _)) = bucket.entries.front() {
            if *front_ts < cutoff {
                bucket.pop_front();
            } else {
                break;
            }
        }
        // O(1) dedup against any in-window entry's event_id (henrypark133
        // must-fix #1 on PR #3635: the previous linear scan held the
        // outer mutex while scanning thousands of entries on the hot
        // path).
        if !bucket.dedup_ids.contains(event_id) {
            bucket.push_back(now, event_id.clone());
        }
        let count = bucket.entries.len() as u32;
        // Drop empty buckets so they can't become zombie LRU-skip keys
        // (codex P1 #2). A bucket gets here empty only if the trim above
        // removed every entry AND `duplicate` was true (so we didn't add
        // a new one).
        if bucket.entries.is_empty() {
            history.remove(key);
        }
        Ok(count)
    }

    fn record_value(
        &self,
        key: &ValueKey,
        event_id: &PredicateEventId,
        now: Instant,
        value: Decimal,
        window: Duration,
    ) -> Result<Decimal, PredicateBackendError> {
        let mut history = match self.value_history.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !history.contains_key(key) && history.len() >= MAX_HISTORY_KEYS {
            evict_lru_value(&mut history, &self.evictions);
        }
        let bucket = history.entry(key.clone()).or_default();
        let cutoff = now.checked_sub(window).unwrap_or(now);
        while let Some((ts, _, _)) = bucket.entries.front() {
            if *ts < cutoff {
                bucket.pop_front();
            } else {
                break;
            }
        }
        if !bucket.dedup_ids.contains(event_id) {
            bucket.push_back(now, value, event_id.clone());
        }
        let sum: Decimal = bucket.entries.iter().map(|(_, v, _)| *v).sum();
        if bucket.entries.is_empty() {
            history.remove(key);
        }
        Ok(sum)
    }

    fn evictions_observed(&self) -> u64 {
        self.evictions.load(AtomicOrdering::Relaxed)
    }
}

/// Evict the entry with the earliest "front" timestamp — that is, the key
/// whose oldest retained sample is older than any other key's oldest
/// sample.
///
/// Empty buckets (entries deque == 0) are the *first* victims —
/// historically they were skipped by `filter_map`, which let zombie
/// keys accumulate past `MAX_HISTORY_KEYS` (codex P1 #2). The record
/// paths now drop empty buckets eagerly, so this is defense-in-depth:
/// if a concurrent code path leaves an empty bucket, the LRU still
/// drains it.
fn evict_lru_invocation(
    history: &mut HashMap<InvocationKey, InvocationBucket>,
    evictions: &AtomicU64,
) {
    // First: any empty bucket?
    let empty_victim = history
        .iter()
        .find(|(_, v)| v.entries.is_empty())
        .map(|(k, _)| k.clone());
    let victim = empty_victim.or_else(|| {
        history
            .iter()
            .filter_map(|(k, v)| v.entries.front().map(|(ts, _)| (k.clone(), *ts)))
            .min_by_key(|(_, ts)| *ts)
            .map(|(k, _)| k)
    });
    if let Some(k) = victim {
        history.remove(&k);
        evictions.fetch_add(1, AtomicOrdering::Relaxed);
    }
}

fn evict_lru_value(history: &mut HashMap<ValueKey, ValueBucket>, evictions: &AtomicU64) {
    let empty_victim = history
        .iter()
        .find(|(_, v)| v.entries.is_empty())
        .map(|(k, _)| k.clone());
    let victim = empty_victim.or_else(|| {
        history
            .iter()
            .filter_map(|(k, v)| v.entries.front().map(|(ts, _, _)| (k.clone(), *ts)))
            .min_by_key(|(_, ts)| *ts)
            .map(|(k, _)| k)
    });
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
        // Test fixture ids are well-formed; use unchecked.
        PredicateEventId::new_unchecked(s)
    }

    /// serrrfirat MEDIUM regression on PR #3635: caller-facing
    /// `PredicateEventId::new` must reject empty + NUL-bearing values
    /// at the type boundary so durable backends can't ever receive
    /// invalid ids by way of a public-field direct assignment on the
    /// hook context.
    #[test]
    fn predicate_event_id_rejects_empty() {
        assert_eq!(PredicateEventId::new(""), Err(PredicateEventIdError::Empty));
    }

    #[test]
    fn predicate_event_id_rejects_nul_bytes() {
        assert_eq!(
            PredicateEventId::new("abc\0def"),
            Err(PredicateEventIdError::ContainsNul)
        );
    }

    #[test]
    fn predicate_event_id_accepts_typical_hex_digest() {
        assert!(
            PredicateEventId::new(
                "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234"
            )
            .is_ok()
        );
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

    /// codex P1 #1 regression: dedup memory must be tied to the window,
    /// not a fixed-size ring. Under high-throughput keys (>256 distinct
    /// in-window events), an event re-emitted from far back in the
    /// window was previously silently re-counted because its event-id
    /// had aged out of the `recent_ids` ring. Now dedup checks against
    /// every in-window entry, so this can't happen.
    #[test]
    fn dedup_memory_covers_full_window_under_high_throughput() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.busy".to_string(),
        };
        let t0 = Instant::now();
        let window = Duration::from_secs(3600);
        // Push 512 distinct events into the bucket within the window.
        // The old implementation capped the dedup ring at 256, so
        // event-0's id had aged out — a replay would have silently
        // counted again. With the new design (dedup vs in-window
        // entries), every replay is a no-op.
        for i in 0..512u32 {
            let _ = backend
                .record_invocation(
                    &key,
                    &ev(&format!("event-{i}")),
                    t0 + Duration::from_millis(i as u64),
                    window,
                )
                .expect("ok");
        }
        let count_after_inserts = backend
            .record_invocation(
                &key,
                &ev("event-fresh"),
                t0 + Duration::from_secs(1),
                window,
            )
            .expect("ok");
        assert_eq!(count_after_inserts, 513);

        // Replay the FIRST event id — should be a no-op because
        // event-0's timestamp is still in the window.
        let count_after_replay = backend
            .record_invocation(&key, &ev("event-0"), t0 + Duration::from_secs(2), window)
            .expect("ok");
        assert_eq!(
            count_after_replay, 513,
            "replay of an event still in the window must be a no-op even after >256 distinct events"
        );
    }

    /// codex P1 #2: in the prior design, `recent_ids` was a separate
    /// ring from `entries`. A duplicate-replay after the window expired
    /// would trim `entries` to empty, then the duplicate check would
    /// hit `recent_ids` (still populated) and skip the new entry —
    /// leaving the bucket empty AND retained, where the LRU search
    /// filtered it out as a zombie. The new design dedupes against
    /// `entries` directly, so once `entries` is empty there are no
    /// dedup hits and the call adds a fresh entry. The unreachable-by-
    /// record-flow empty-bucket case is still defended at the LRU
    /// level (see `lru_evicts_empty_buckets_first`).
    ///
    /// LRU defense-in-depth: even if some path leaves an empty bucket,
    /// the LRU eviction must prefer it as the first victim instead of
    /// skipping it (which would let zombies accumulate).
    #[test]
    fn lru_evicts_empty_buckets_first() {
        let backend = InMemoryPredicateStateBackend::new();
        let empty_key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.empty".to_string(),
        };
        let live_key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.live".to_string(),
        };
        // Manually craft an empty bucket alongside a live one to
        // simulate the bug condition.
        {
            let mut map = backend.invocation_history.lock().expect("ok");
            map.insert(empty_key.clone(), InvocationBucket::default());
            let mut live = InvocationBucket::default();
            live.entries.push_back((Instant::now(), ev("live-evt")));
            map.insert(live_key.clone(), live);
        }
        // Force an LRU pass.
        {
            let mut map = backend.invocation_history.lock().expect("ok");
            evict_lru_invocation(&mut map, &backend.evictions);
        }
        let map = backend.invocation_history.lock().expect("ok");
        assert!(
            !map.contains_key(&empty_key),
            "empty bucket should be the first LRU victim"
        );
        assert!(
            map.contains_key(&live_key),
            "live bucket should be retained"
        );
        assert_eq!(backend.evictions_observed(), 1);
    }

    /// henrypark133 missing-coverage #1 on PR #3635: prove the atomic
    /// record-and-read contract holds under concurrent writers. N threads
    /// each call `record_invocation` once with a distinct `event_id`; the
    /// final observed count must equal N (no lost-update race).
    #[test]
    fn in_memory_record_invocation_is_atomic_under_concurrent_writers() {
        use std::sync::Arc as StdArc;
        use std::thread;
        let backend = StdArc::new(InMemoryPredicateStateBackend::new());
        let key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.concurrent".to_string(),
        };
        let now = Instant::now();
        const N: usize = 32;
        let handles: Vec<_> = (0..N)
            .map(|i| {
                let backend = StdArc::clone(&backend);
                let key = key.clone();
                let ev = ev(&format!("event-{i}"));
                thread::spawn(move || {
                    backend
                        .record_invocation(&key, &ev, now, Duration::from_secs(60))
                        .expect("ok")
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread joined");
        }
        // Re-read via a no-op record with a duplicate id to observe the
        // final count without inserting a new entry.
        let final_count = backend
            .record_invocation(&key, &ev("event-0"), now, Duration::from_secs(60))
            .expect("ok");
        assert_eq!(
            final_count as usize, N,
            "N concurrent distinct-id writes must each be counted exactly once \
             (atomic record-and-read contract)"
        );
    }
}
