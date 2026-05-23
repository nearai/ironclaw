//! Pluggable backend for predicate sliding-window state.
//!
//! The [`PredicateEvaluator`] in [`crate::evaluator`] delegates its
//! counter / value-sum bookkeeping to a [`PredicateStateBackend`]. The
//! default backend ([`InMemoryPredicateStateBackend`]) preserves the
//! existing in-process semantics; future durable backends
//! (Postgres-backed, libSQL-backed) implement the same trait so the
//! evaluator's predicate logic stays backend-agnostic.
//!
//! # Visibility — stable public contract
//!
//! [`PredicateStateBackend`] and its supporting types ([`InvocationKey`],
//! [`ValueKey`], [`PredicateBackendError`]) are **public** and form the
//! stable contract that durable backends (Postgres-backed, libSQL-backed)
//! implement out-of-crate. The contract was widened from `pub(crate)` in
//! the durable-backend split (PR 1/4): the trait is now `async` (durable
//! backends are async; a sync trait would force `block_on` on the dispatch
//! hot path) and the clock is [`chrono::DateTime<Utc>`] rather than
//! [`Instant`] (`Instant` is not serializable across processes).
//!
//! Backends are exercised against the shared [`contract`] harness so every
//! impl is proven to honor the same isolation / dedup / window invariants
//! by construction, not per-impl.
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
//! # Async trait, serializable clock
//!
//! The trait is **async** ([`async_trait`]) so durable backends can do
//! `tokio-postgres` / libSQL I/O without blocking the dispatch hot path.
//! The in-memory backend completes synchronously inside the async body.
//!
//! The clock is [`chrono::DateTime<Utc>`] rather than [`Instant`]: a
//! serializable wall-clock timestamp is required for durable rows that
//! survive process restarts and are written by one host and read by
//! another. Window trimming is age-based — an entry is trimmed when its
//! timestamp is strictly older than `now - window`. Because
//! `DateTime<Utc>` arithmetic saturates rather than panicking, the
//! `Instant::checked_sub` underflow hazard of the prior design does not
//! arise.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::TenantId;
use rust_decimal::Decimal;

use crate::identity::HookId;

/// Durable libSQL-backed [`PredicateStateBackend`] (durable-backend PR
/// 3/4). Gated on the `libsql` cargo feature so default / postgres / no-DB
/// builds don't pull in the `libsql` crate.
#[cfg(feature = "libsql")]
#[path = "predicate_state_libsql.rs"]
pub mod libsql;

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
    /// External callers must use [`Self::new`].
    ///
    /// Visibility is `pub(crate)` (henrypark133 HIGH on PR #3635 5-19
    /// review): external callers can otherwise bypass the durable
    /// UNIQUE-constraint format invariants enforced by [`Self::new`]
    /// (non-empty, NUL-free). Restricting the constructor forces every
    /// caller crossing the crate boundary through validated construction.
    pub(crate) fn new_unchecked(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Synthesize a per-call-unique event id. Used by the evaluator's
    /// `resolve_event_id` fallback only when the caller has not supplied
    /// a stable id via [`crate::points::BeforeCapabilityHookContext::caller_event_id`];
    /// see that path's documentation for the load-bearing semantics.
    ///
    /// The id is the hex digest of `(hook_id, capability_name,
    /// thread-local seed, process-local counter)`. The counter and
    /// thread-local seed together guarantee uniqueness across calls even
    /// when `(hook, ctx)` are bit-identical (which happens routinely in
    /// tests and can happen in production under tight loops on coarse
    /// clocks).
    ///
    /// # Why `arguments_digest` is NOT mixed in
    ///
    /// An earlier revision included the caller's `arguments_digest` in
    /// the hash. That made the resulting 64-char hex id an **equality
    /// oracle** for argument shape: two invocations with bit-identical
    /// arguments (same tenant or otherwise) would yield colliding hash
    /// inputs, which a downstream observer of the event id (audit logs,
    /// durable backend rows visible cross-tenant) could use to correlate
    /// tenant traffic patterns. The replay-dedup contract for durable
    /// backends is driven by the caller-supplied `caller_event_id`, NOT
    /// by the synth path — synth only needs to be per-call unique, not
    /// content-addressed. Dropping `arguments_digest` from the hash
    /// closes the oracle without weakening any property the synth path
    /// is responsible for (henrypark133 LOW on PR #3635 5-19 review).
    ///
    /// The argument shape is `(&[u8], &str)` rather than
    /// `&BeforeCapabilityHookContext` so this function can live in
    /// `predicate_state` next to its consumer (the backend) without
    /// inverting the module dependency graph
    /// (`predicate_state` is a leaf below `points`).
    ///
    /// Lives here rather than in `evaluator` because the id format —
    /// 64-char lowercase hex, no NUL, never empty — is part of the
    /// backend's durable contract (henrypark133 nit on PR #3635).
    pub(crate) fn synth(hook_id_bytes: &[u8], capability_name: &str) -> Self {
        use std::fmt::Write;
        // Process-global monotone counter. Documented hotspot
        // (henrypark133 MED on PR #3635 5-19 review): under very high
        // synth rates across all cores this AtomicU64 is a cache-line
        // contention point. The synth path is only reached when callers
        // omit `caller_event_id` (legacy / test paths), so production
        // durable-backend traffic should not contend here in practice.
        // We additionally combine with a thread-local nonce so the
        // global counter does not need to be the sole uniqueness source
        // — N threads can each advance their thread-local nonce without
        // forcing a cross-core invalidation.
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        thread_local! {
            static THREAD_NONCE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        }
        let seq = COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let thread_seq = THREAD_NONCE.with(|n| {
            let v = n.get().wrapping_add(1);
            n.set(v);
            v
        });

        let mut hasher = blake3::Hasher::new();
        hasher.update(hook_id_bytes);
        hasher.update(capability_name.as_bytes());
        hasher.update(&seq.to_le_bytes());
        hasher.update(&thread_seq.to_le_bytes());
        let digest = hasher.finalize();
        let mut s = String::with_capacity(64);
        for byte in digest.as_bytes() {
            // std::fmt::Write for String is infallible; discard the
            // Result rather than `.expect()` so this stays out of the
            // "no panics in production code" CI check.
            let _ = write!(s, "{byte:02x}");
        }
        // Synth output is always a 64-char hex digest — no NUL, never
        // empty — so skip the per-call validation in `Self::new`.
        Self::new_unchecked(s)
    }
}

/// Error surface for the backend. Durable backends use this to signal
/// transient failures (connection lost, DB unavailable); the in-memory
/// backend never returns errors.
#[derive(Debug, thiserror::Error)]
pub enum PredicateBackendError {
    #[error("predicate state backend is unavailable: {0}")]
    #[allow(dead_code)] // populated by future durable backends; in-memory is infallible
    Unavailable(String),
}

/// Backend for the predicate evaluator's sliding-window state.
///
/// Each method is bracketed by `now` so tests can drive a deterministic
/// clock; production callers pass [`Utc::now()`].
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
///
/// # Cross-process replay limits (in-memory backend)
///
/// Replay dedup on [`InMemoryPredicateStateBackend`] is **process-local**.
/// An `event_id` recorded on host A is unknown to host B: replaying the
/// same `event_id` against host B will count as a fresh invocation,
/// double-counting against the logical cap. The in-memory backend
/// therefore does NOT defend against multi-host replay; that property
/// requires the durable backend (see successor doc
/// `03-persistent-counter.md`), where the SQL `UNIQUE` constraint on
/// `(tenant_id, hook_id, capability[, field], event_id)` enforces dedup
/// across every host pointing at the same database
/// (henrypark133 security note on PR #3635). Threat-model finding D5a
/// also captures the high-cardinality-key LRU-eviction variant that lets
/// an attacker reset another tenant's counter on the in-memory backend.
#[async_trait]
pub trait PredicateStateBackend: Send + Sync {
    /// Record an invocation at `now` against `key` (idempotent against
    /// `event_id`) and return the resulting in-window count after
    /// trimming entries older than `window`. Atomic against concurrent
    /// writers.
    async fn record_invocation(
        &self,
        key: &InvocationKey,
        event_id: &PredicateEventId,
        now: DateTime<Utc>,
        window: Duration,
    ) -> Result<u32, PredicateBackendError>;

    /// Record a numeric value at `now` against `key` (idempotent
    /// against `event_id`) and return the resulting in-window sum
    /// after trimming. Atomic against concurrent writers.
    async fn record_value(
        &self,
        key: &ValueKey,
        event_id: &PredicateEventId,
        now: DateTime<Utc>,
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
    async fn evict_older_than(&self, _cutoff: DateTime<Utc>) -> Result<u64, PredicateBackendError> {
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
    entries: VecDeque<(DateTime<Utc>, PredicateEventId)>,
    dedup_ids: HashSet<PredicateEventId>,
}

impl InvocationBucket {
    fn pop_front(&mut self) {
        if let Some((_, id)) = self.entries.pop_front() {
            self.dedup_ids.remove(&id);
        }
    }

    fn push_back(&mut self, ts: DateTime<Utc>, event_id: PredicateEventId) {
        self.dedup_ids.insert(event_id.clone());
        self.entries.push_back((ts, event_id));
    }
}

/// Per-key sliding-window bucket for NumericSum predicates. Holds the
/// FIFO of `(timestamp, value, event_id)` entries, a companion dedup set,
/// AND an incrementally-maintained `running_sum` so the public sum is
/// O(1) per call (henrypark133 HIGH on PR #3635 5-19 review).
///
/// Invariant: `running_sum == entries.iter().map(|(_, v, _)| v).sum()`.
/// Maintained by `push_back` (add) and `pop_front` (subtract) — the only
/// two ways the deque mutates.
#[derive(Debug, Default)]
struct ValueBucket {
    entries: VecDeque<(DateTime<Utc>, Decimal, PredicateEventId)>,
    dedup_ids: HashSet<PredicateEventId>,
    running_sum: Decimal,
}

impl ValueBucket {
    fn pop_front(&mut self) {
        if let Some((_, v, id)) = self.entries.pop_front() {
            self.dedup_ids.remove(&id);
            self.running_sum -= v;
        }
    }

    fn push_back(&mut self, ts: DateTime<Utc>, value: Decimal, event_id: PredicateEventId) {
        self.dedup_ids.insert(event_id.clone());
        self.running_sum += value;
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
pub struct InMemoryPredicateStateBackend {
    invocation_history: Mutex<HashMap<InvocationKey, InvocationBucket>>,
    value_history: Mutex<HashMap<ValueKey, ValueBucket>>,
    evictions: AtomicU64,
}

/// Maximum number of distinct keys retained per history map. Bounds the
/// in-memory backend's memory footprint against threat-model finding **D5**.
pub const MAX_HISTORY_KEYS: usize = 8_192;

/// Per-tenant ceiling on distinct keys held in either history map.
/// Defends against the cross-tenant LRU-reset attack (henrypark133 MED
/// on PR #3635 5-19 review): without a per-tenant quota, a noisy tenant
/// can grow its key footprint to evict a quiet tenant's bucket, which
/// resets that tenant's rate-limit counter on the next `record_*` call.
/// With the quota, a tenant that exceeds the cap evicts ITS OWN
/// oldest-front bucket first, leaving other tenants' buckets untouched.
///
/// Chosen as `MAX_HISTORY_KEYS / 4`: a single tenant cannot consume more
/// than 25 % of the global cap, so at least four tenants always fit
/// concurrently regardless of insertion order. This is a defense-in-
/// depth measure for the in-memory backend; the durable backend
/// (separate PR) enforces per-tenant scoping at the schema level via
/// PRIMARY KEY (tenant_id, …).
pub const MAX_KEYS_PER_TENANT: usize = MAX_HISTORY_KEYS / 4;

/// Maximum number of samples retained per `(hook, capability, tenant[, field])`
/// key in either sliding-window history. Without this cap, an installed hook
/// could declare a very large window on a hot capability and force the
/// backend to retain every invocation in the window, exhausting memory under
/// attacker-triggered hot capabilities (threat-model finding **D5**).
///
/// Once this cap is reached for a key, the oldest samples are dropped to make
/// room for the new one — the predicate continues to evaluate against the
/// most-recent `MAX_SAMPLES_PER_KEY` samples in the window, which is the
/// conservative bound for a rate / value cap.
///
/// Ported from the pre-extraction inline enforcement in `evaluator.rs` that
/// PR #3573 round-3 review introduced; the predicate-state extraction in
/// PR #3635 moved the bookkeeping into this trait, so the cap must live with
/// the bookkeeping (durable backends must enforce the same bound — see
/// trait-level rustdoc on [`PredicateStateBackend`]).
pub const MAX_SAMPLES_PER_KEY: usize = 4_096;

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

/// Compute the sliding-window cutoff `now - window` as a wall-clock
/// timestamp. `window` is a [`std::time::Duration`] (always non-negative);
/// `chrono::Duration::from_std` only fails if the window exceeds chrono's
/// `i64`-millisecond range (~292 million years), which no parsed predicate
/// window can reach — in that pathological case we saturate to `now`,
/// meaning nothing is trimmed (conservative for a rate/value cap). Unlike
/// the prior `Instant::checked_sub`, `DateTime<Utc>` subtraction never
/// panics or underflows.
fn window_cutoff(now: DateTime<Utc>, window: Duration) -> DateTime<Utc> {
    match chrono::Duration::from_std(window) {
        Ok(d) => now.checked_sub_signed(d).unwrap_or(now),
        Err(_) => now,
    }
}

#[async_trait]
impl PredicateStateBackend for InMemoryPredicateStateBackend {
    async fn record_invocation(
        &self,
        key: &InvocationKey,
        event_id: &PredicateEventId,
        now: DateTime<Utc>,
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
        if !history.contains_key(key) {
            // Per-tenant quota: if this tenant already holds the cap,
            // evict their oldest-front bucket FIRST (not a global LRU
            // pass) so noisy tenants can't push quiet tenants out.
            // henrypark133 MED on PR #3635 5-19 review.
            let tenant_count = history
                .keys()
                .filter(|k| k.tenant_id == key.tenant_id)
                .count();
            if tenant_count >= MAX_KEYS_PER_TENANT {
                evict_lru_invocation_for_tenant(&mut history, &key.tenant_id, &self.evictions);
            } else if history.len() >= MAX_HISTORY_KEYS {
                evict_lru_invocation(&mut history, &self.evictions);
            }
        }
        let bucket = history.entry(key.clone()).or_default();
        let cutoff = window_cutoff(now, window);
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
            // Per-key sample cap: drop the oldest sample(s) to make room.
            // Bounds per-key memory under attacker-triggered hot capabilities
            // (threat-model finding D5). Ported from the pre-extraction
            // inline enforcement in `evaluator.rs` (PR #3573 round 3) which
            // the predicate-state extraction missed.
            while bucket.entries.len() >= MAX_SAMPLES_PER_KEY {
                bucket.pop_front();
            }
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

    async fn record_value(
        &self,
        key: &ValueKey,
        event_id: &PredicateEventId,
        now: DateTime<Utc>,
        value: Decimal,
        window: Duration,
    ) -> Result<Decimal, PredicateBackendError> {
        let mut history = match self.value_history.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !history.contains_key(key) {
            let tenant_count = history
                .keys()
                .filter(|k| k.tenant_id == key.tenant_id)
                .count();
            if tenant_count >= MAX_KEYS_PER_TENANT {
                evict_lru_value_for_tenant(&mut history, &key.tenant_id, &self.evictions);
            } else if history.len() >= MAX_HISTORY_KEYS {
                evict_lru_value(&mut history, &self.evictions);
            }
        }
        let bucket = history.entry(key.clone()).or_default();
        let cutoff = window_cutoff(now, window);
        while let Some((ts, _, _)) = bucket.entries.front() {
            if *ts < cutoff {
                bucket.pop_front();
            } else {
                break;
            }
        }
        if !bucket.dedup_ids.contains(event_id) {
            // Per-key sample cap: drop oldest sample(s). `pop_front`
            // decrements `running_sum` so the incremental invariant
            // (`running_sum == sum(entries values)`) is preserved across
            // eviction (threat-model finding D5). Ported from the
            // pre-extraction inline enforcement in `evaluator.rs`.
            while bucket.entries.len() >= MAX_SAMPLES_PER_KEY {
                bucket.pop_front();
            }
            bucket.push_back(now, value, event_id.clone());
        }
        // O(1): the bucket maintains `running_sum` incrementally on
        // push/pop, so we don't re-walk the deque on every call
        // (henrypark133 HIGH on PR #3635 5-19 review). Snapshot before
        // the potential empty-bucket removal below.
        let sum = bucket.running_sum;
        if bucket.entries.is_empty() {
            history.remove(key);
        }
        Ok(sum)
    }

    fn evictions_observed(&self) -> u64 {
        self.evictions.load(AtomicOrdering::Relaxed)
    }

    /// Drop all entries with `timestamp < cutoff` from every bucket, and
    /// remove buckets that become empty. Returns the total number of
    /// entries dropped across both history maps (invocation + value).
    ///
    /// Idle keys are otherwise only reclaimed when the bucket itself is
    /// touched by a `record_*` call or the LRU evicts at the
    /// `MAX_HISTORY_KEYS` cap. A reaper task pointed at the slowest
    /// configured window keeps idle memory bounded even when no traffic
    /// is hitting those keys (henrypark133 MED on PR #3635 5-19 review:
    /// the default no-op was misleading because callers expected it to
    /// reclaim memory).
    async fn evict_older_than(&self, cutoff: DateTime<Utc>) -> Result<u64, PredicateBackendError> {
        let mut dropped: u64 = 0;
        {
            let mut history = match self.invocation_history.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            history.retain(|_, bucket| {
                while let Some((front_ts, _)) = bucket.entries.front() {
                    if *front_ts < cutoff {
                        bucket.pop_front();
                        dropped += 1;
                    } else {
                        break;
                    }
                }
                !bucket.entries.is_empty()
            });
        }
        {
            let mut history = match self.value_history.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            history.retain(|_, bucket| {
                while let Some((front_ts, _, _)) = bucket.entries.front() {
                    if *front_ts < cutoff {
                        bucket.pop_front();
                        dropped += 1;
                    } else {
                        break;
                    }
                }
                !bucket.entries.is_empty()
            });
        }
        Ok(dropped)
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

/// Tenant-scoped variant: evict the oldest-front bucket BELONGING TO
/// `tenant_id`. Used when a single tenant hits [`MAX_KEYS_PER_TENANT`]
/// so the eviction stays within that tenant's footprint and cannot
/// reach into another tenant's buckets (henrypark133 MED on PR #3635
/// 5-19 review).
fn evict_lru_invocation_for_tenant(
    history: &mut HashMap<InvocationKey, InvocationBucket>,
    tenant_id: &TenantId,
    evictions: &AtomicU64,
) {
    let empty_victim = history
        .iter()
        .find(|(k, v)| k.tenant_id == *tenant_id && v.entries.is_empty())
        .map(|(k, _)| k.clone());
    let victim = empty_victim.or_else(|| {
        history
            .iter()
            .filter(|(k, _)| k.tenant_id == *tenant_id)
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

/// Tenant-scoped variant for the value-sum map. Same rationale as
/// [`evict_lru_invocation_for_tenant`].
fn evict_lru_value_for_tenant(
    history: &mut HashMap<ValueKey, ValueBucket>,
    tenant_id: &TenantId,
    evictions: &AtomicU64,
) {
    let empty_victim = history
        .iter()
        .find(|(k, v)| k.tenant_id == *tenant_id && v.entries.is_empty())
        .map(|(k, _)| k.clone());
    let victim = empty_victim.or_else(|| {
        history
            .iter()
            .filter(|(k, _)| k.tenant_id == *tenant_id)
            .filter_map(|(k, v)| v.entries.front().map(|(ts, _, _)| (k.clone(), *ts)))
            .min_by_key(|(_, ts)| *ts)
            .map(|(k, _)| k)
    });
    if let Some(k) = victim {
        history.remove(&k);
        evictions.fetch_add(1, AtomicOrdering::Relaxed);
    }
}

/// Trait-level contract test harness for [`PredicateStateBackend`].
///
/// This establishes the **scaffolding pattern** (mirrors
/// `ironclaw_memory::contract_tests`, #3918): every invariant the trait
/// promises is defined once here as a `pub async fn` taking a factory
/// closure, and every impl wires the suite once. Durable backends added
/// in the durable-backend split (PRs 2/3 — Postgres, libSQL) drop into
/// the same suite so isolation / dedup / window / atomicity invariants
/// are proven for every impl by construction, not per-impl.
///
/// The in-memory backend is wired below via [`contract_test!`]; that is
/// the proof that the harness shape works with one backend before the
/// durable impls land.
///
/// Gated on `any(test, feature = "contract-tests")` so out-of-crate
/// durable backends can depend on `ironclaw_hooks` with the
/// `contract-tests` feature and run the same suite against their impl.
#[cfg(any(test, feature = "contract-tests"))]
pub mod contract {
    use super::*;

    use crate::identity::{ExtensionId, HookLocalId, HookVersion};

    /// Factory closure shape every contract takes. Must return a fresh,
    /// empty backend — contracts assume nothing leaks between calls.
    fn hook_id() -> HookId {
        HookId::derive(
            &ExtensionId::new("ext").expect("ext id is valid"),
            "1.0",
            &HookLocalId::new("h").expect("hook local id is valid"),
            HookVersion::ONE,
        )
    }

    fn tenant_named(name: &str) -> TenantId {
        TenantId::new(name).expect("valid tenant id")
    }

    fn ev(s: &str) -> PredicateEventId {
        PredicateEventId::new_unchecked(s)
    }

    fn inv_key(tenant: &str, capability: &str) -> InvocationKey {
        InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant_named(tenant),
            capability: capability.to_string(),
        }
    }

    fn val_key(tenant: &str, capability: &str, field: &str) -> ValueKey {
        ValueKey {
            hook_id: hook_id(),
            tenant_id: tenant_named(tenant),
            capability: capability.to_string(),
            field: field.to_string(),
        }
    }

    /// Shared fixed wall-clock base so contracts are deterministic.
    fn base() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("valid fixed timestamp")
    }

    fn at(secs: i64) -> DateTime<Utc> {
        base() + chrono::Duration::seconds(secs)
    }

    /// Contract: invocations within the window accumulate.
    pub async fn invocation_counts_within_window<B, F>(factory: F)
    where
        B: PredicateStateBackend,
        F: Fn() -> B,
    {
        let backend = factory();
        let key = inv_key("alpha", "cap.x");
        let c1 = backend
            .record_invocation(&key, &ev("e1"), at(0), Duration::from_secs(60))
            .await
            .expect("ok");
        let c2 = backend
            .record_invocation(&key, &ev("e2"), at(1), Duration::from_secs(60))
            .await
            .expect("ok");
        assert_eq!(c1, 1);
        assert_eq!(c2, 2);
    }

    /// Contract: entries older than the window are trimmed.
    pub async fn invocation_trims_outside_window<B, F>(factory: F)
    where
        B: PredicateStateBackend,
        F: Fn() -> B,
    {
        let backend = factory();
        let key = inv_key("alpha", "cap.x");
        let _ = backend
            .record_invocation(&key, &ev("e1"), at(0), Duration::from_secs(10))
            .await
            .expect("ok");
        let count = backend
            .record_invocation(&key, &ev("e2"), at(100), Duration::from_secs(10))
            .await
            .expect("ok");
        assert_eq!(count, 1, "earlier entry should be trimmed");
    }

    /// Contract: numeric values within the window sum.
    pub async fn value_sums_within_window<B, F>(factory: F)
    where
        B: PredicateStateBackend,
        F: Fn() -> B,
    {
        let backend = factory();
        let key = val_key("alpha", "cap.x", "amount");
        let s1 = backend
            .record_value(
                &key,
                &ev("e1"),
                at(0),
                Decimal::from(50),
                Duration::from_secs(60),
            )
            .await
            .expect("ok");
        let s2 = backend
            .record_value(
                &key,
                &ev("e2"),
                at(1),
                Decimal::from(75),
                Duration::from_secs(60),
            )
            .await
            .expect("ok");
        assert_eq!(s1, Decimal::from(50));
        assert_eq!(s2, Decimal::from(125));
    }

    /// Contract: one tenant's counter never inherits another's increments.
    pub async fn tenant_isolation<B, F>(factory: F)
    where
        B: PredicateStateBackend,
        F: Fn() -> B,
    {
        let backend = factory();
        let alpha = inv_key("alpha", "cap.x");
        let beta = inv_key("beta", "cap.x");
        for e in ["e1", "e2", "e3"] {
            backend
                .record_invocation(&alpha, &ev(e), at(0), Duration::from_secs(60))
                .await
                .expect("ok");
        }
        let beta_count = backend
            .record_invocation(&beta, &ev("b1"), at(0), Duration::from_secs(60))
            .await
            .expect("ok");
        assert_eq!(
            beta_count, 1,
            "tenant β's counter must not inherit α's increments"
        );
    }

    /// Contract: re-emitting a recorded `event_id` is a no-op against the
    /// invocation count (replay refusal — load-bearing, codex Critical on
    /// PR #3635).
    pub async fn duplicate_event_id_is_noop_for_invocations<B, F>(factory: F)
    where
        B: PredicateStateBackend,
        F: Fn() -> B,
    {
        let backend = factory();
        let key = inv_key("alpha", "cap.x");
        let c1 = backend
            .record_invocation(&key, &ev("event-A"), at(0), Duration::from_secs(60))
            .await
            .expect("ok");
        let c2 = backend
            .record_invocation(&key, &ev("event-A"), at(1), Duration::from_secs(60))
            .await
            .expect("ok");
        let c3 = backend
            .record_invocation(&key, &ev("event-B"), at(2), Duration::from_secs(60))
            .await
            .expect("ok");
        assert_eq!(c1, 1);
        assert_eq!(c2, 1, "duplicate event-A must not increment");
        assert_eq!(c3, 2, "distinct event-B advances the count");
    }

    /// Contract: replayed `event_id` is a no-op against the value sum.
    pub async fn duplicate_event_id_is_noop_for_values<B, F>(factory: F)
    where
        B: PredicateStateBackend,
        F: Fn() -> B,
    {
        let backend = factory();
        let key = val_key("alpha", "cap.x", "amount");
        let s1 = backend
            .record_value(
                &key,
                &ev("event-A"),
                at(0),
                Decimal::from(50),
                Duration::from_secs(60),
            )
            .await
            .expect("ok");
        let s2 = backend
            .record_value(
                &key,
                &ev("event-A"),
                at(1),
                Decimal::from(50),
                Duration::from_secs(60),
            )
            .await
            .expect("ok");
        let s3 = backend
            .record_value(
                &key,
                &ev("event-B"),
                at(2),
                Decimal::from(75),
                Duration::from_secs(60),
            )
            .await
            .expect("ok");
        assert_eq!(s1, Decimal::from(50));
        assert_eq!(
            s2,
            Decimal::from(50),
            "duplicate event-A must not double-count"
        );
        assert_eq!(s3, Decimal::from(125));
    }

    /// Contract: an entry whose timestamp equals the cutoff is RETAINED
    /// (`< cutoff` trim, not `<=`).
    pub async fn invocation_retains_entry_at_exact_window_cutoff<B, F>(factory: F)
    where
        B: PredicateStateBackend,
        F: Fn() -> B,
    {
        let backend = factory();
        let key = inv_key("alpha", "cap.boundary");
        let window = Duration::from_secs(60);
        let _ = backend
            .record_invocation(&key, &ev("e-at-t0"), at(0), window)
            .await
            .expect("ok");
        let at_cutoff = backend
            .record_invocation(&key, &ev("e-at-boundary"), at(60), window)
            .await
            .expect("ok");
        assert_eq!(
            at_cutoff, 2,
            "entry whose timestamp equals the cutoff is retained (< cutoff, not <=)"
        );
    }

    /// Contract: dedup is isolated across the invocation and value maps —
    /// the same `event_id` in both must not collide.
    pub async fn event_id_dedup_isolated_across_maps<B, F>(factory: F)
    where
        B: PredicateStateBackend,
        F: Fn() -> B,
    {
        let backend = factory();
        let inv = inv_key("alpha", "cap.cross");
        let val = val_key("alpha", "cap.cross", "amount");
        let shared = ev("shared-event-id");
        let inv_count = backend
            .record_invocation(&inv, &shared, at(0), Duration::from_secs(60))
            .await
            .expect("ok");
        let val_sum = backend
            .record_value(
                &val,
                &shared,
                at(0),
                Decimal::from(42),
                Duration::from_secs(60),
            )
            .await
            .expect("ok");
        assert_eq!(
            inv_count, 1,
            "value map's dedup must not pre-empt invocation"
        );
        assert_eq!(
            val_sum,
            Decimal::from(42),
            "invocation map's dedup must not pre-empt value"
        );
    }

    /// Run every contract against `factory`. Per-impl test files invoke
    /// this via [`contract_test!`].
    #[macro_export]
    macro_rules! predicate_backend_contract_test {
        ($label:ident, $factory:expr) => {
            mod $label {
                #[tokio::test]
                async fn invocation_counts_within_window() {
                    $crate::predicate_state::contract::invocation_counts_within_window($factory)
                        .await;
                }
                #[tokio::test]
                async fn invocation_trims_outside_window() {
                    $crate::predicate_state::contract::invocation_trims_outside_window($factory)
                        .await;
                }
                #[tokio::test]
                async fn value_sums_within_window() {
                    $crate::predicate_state::contract::value_sums_within_window($factory).await;
                }
                #[tokio::test]
                async fn tenant_isolation() {
                    $crate::predicate_state::contract::tenant_isolation($factory).await;
                }
                #[tokio::test]
                async fn duplicate_event_id_is_noop_for_invocations() {
                    $crate::predicate_state::contract::duplicate_event_id_is_noop_for_invocations(
                        $factory,
                    )
                    .await;
                }
                #[tokio::test]
                async fn duplicate_event_id_is_noop_for_values() {
                    $crate::predicate_state::contract::duplicate_event_id_is_noop_for_values(
                        $factory,
                    )
                    .await;
                }
                #[tokio::test]
                async fn invocation_retains_entry_at_exact_window_cutoff() {
                    $crate::predicate_state::contract::invocation_retains_entry_at_exact_window_cutoff(
                        $factory,
                    )
                    .await;
                }
                #[tokio::test]
                async fn event_id_dedup_isolated_across_maps() {
                    $crate::predicate_state::contract::event_id_dedup_isolated_across_maps($factory)
                        .await;
                }
            }
        };
    }
}

// Wire the in-memory backend through the shared contract suite. This is
// the proof that the harness shape works with one backend before the
// durable impls (PRs 2/3) drop in.
#[cfg(test)]
crate::predicate_backend_contract_test!(
    in_memory,
    crate::predicate_state::InMemoryPredicateStateBackend::new
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ExtensionId, HookLocalId, HookVersion};
    use rust_decimal::Decimal;

    fn hook_id() -> HookId {
        HookId::derive(
            &ExtensionId::new("ext").expect("ext id is valid"),
            "1.0",
            &HookLocalId::new("h").expect("hook local id is valid"),
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

    /// Fixed wall-clock base so window arithmetic is deterministic. The
    /// clock is now `DateTime<Utc>` (serializable across processes) rather
    /// than `Instant`; `+ chrono::Duration` advances it without the
    /// `Instant::checked_sub` underflow hazard of the prior design.
    fn base() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("valid fixed timestamp")
    }

    fn at_secs(secs: i64) -> DateTime<Utc> {
        base() + chrono::Duration::seconds(secs)
    }

    fn at_millis(ms: i64) -> DateTime<Utc> {
        base() + chrono::Duration::milliseconds(ms)
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

    /// henrypark133 nit #5 on PR #3635: pin the synth output shape. The
    /// 64-char lowercase-hex format is part of the durable backend's
    /// expectation (e.g. Postgres `uuid` UNIQUE constraint accepts hex)
    /// — a refactor that silently changes the length or character set
    /// would break that contract without surfacing a test failure.
    #[test]
    fn synth_event_id_is_64_char_lowercase_hex() {
        let id = PredicateEventId::synth(b"hookid-bytes", "cap.x");
        let s = id.as_str();
        assert_eq!(s.len(), 64, "synth output must be exactly 64 chars");
        assert!(
            s.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "synth output must be lowercase hex: {s}"
        );
    }

    /// henrypark133 LOW on PR #3635 5-19 review (equality-oracle closure):
    /// two calls with bit-identical `(hook_id_bytes, capability_name)` must
    /// produce DISTINCT synth ids.
    #[test]
    fn synth_diverges_on_identical_inputs() {
        let a = PredicateEventId::synth(b"hookid-bytes", "cap.x");
        let b = PredicateEventId::synth(b"hookid-bytes", "cap.x");
        assert_ne!(
            a.as_str(),
            b.as_str(),
            "synth must not be content-addressable: identical inputs would leak \
             argument-shape equality across tenants"
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

    /// codex P1 #1 regression: dedup memory must be tied to the window,
    /// not a fixed-size ring. Under high-throughput keys (>256 distinct
    /// in-window events), an event re-emitted from far back in the
    /// window must still be deduped.
    #[tokio::test]
    async fn dedup_memory_covers_full_window_under_high_throughput() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.busy".to_string(),
        };
        let window = Duration::from_secs(3600);
        for i in 0..512i64 {
            let _ = backend
                .record_invocation(&key, &ev(&format!("event-{i}")), at_millis(i), window)
                .await
                .expect("ok");
        }
        let count_after_inserts = backend
            .record_invocation(&key, &ev("event-fresh"), at_secs(1), window)
            .await
            .expect("ok");
        assert_eq!(count_after_inserts, 513);

        // Replay the FIRST event id — should be a no-op because event-0's
        // timestamp is still in the window.
        let count_after_replay = backend
            .record_invocation(&key, &ev("event-0"), at_secs(2), window)
            .await
            .expect("ok");
        assert_eq!(
            count_after_replay, 513,
            "replay of an event still in the window must be a no-op even after >256 distinct events"
        );
    }

    /// codex P1 #2 / LRU defense-in-depth: even if some path leaves an
    /// empty bucket, the LRU eviction must prefer it as the first victim.
    #[tokio::test]
    async fn lru_evicts_empty_buckets_first() {
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
        {
            let mut map = backend.invocation_history.lock().expect("ok");
            map.insert(empty_key.clone(), InvocationBucket::default());
            let mut live = InvocationBucket::default();
            live.entries.push_back((at_secs(0), ev("live-evt")));
            map.insert(live_key.clone(), live);
        }
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

    /// henrypark133 missing-coverage on PR #3635: drive the per-tenant
    /// eviction path through the public `record_invocation` API.
    #[tokio::test]
    async fn per_tenant_quota_holds_single_tenant_at_its_cap() {
        let backend = InMemoryPredicateStateBackend::new();
        for i in 0..=MAX_KEYS_PER_TENANT {
            let key = InvocationKey {
                hook_id: hook_id(),
                tenant_id: tenant(),
                capability: format!("cap.{i}"),
            };
            let _ = backend
                .record_invocation(
                    &key,
                    &ev(&format!("e{i}")),
                    at_secs(0),
                    Duration::from_secs(60),
                )
                .await
                .expect("record ok");
        }
        let map = backend.invocation_history.lock().expect("lock ok");
        assert_eq!(
            map.len(),
            MAX_KEYS_PER_TENANT,
            "per-tenant quota must hold the map at MAX_KEYS_PER_TENANT for a single tenant"
        );
        drop(map);
        assert!(
            backend.evictions_observed() >= 1,
            "evictions counter must advance when the per-tenant cap is hit via the public API"
        );
    }

    /// henrypark133 MED on PR #3635 5-19 review: the per-tenant quota must
    /// guarantee that a noisy tenant cannot evict another tenant's bucket.
    #[tokio::test]
    async fn per_tenant_quota_isolates_tenants_from_each_other() {
        let backend = InMemoryPredicateStateBackend::new();
        let alpha = TenantId::new("alpha").expect("ok");
        let beta = TenantId::new("beta").expect("ok");

        let beta_key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: beta.clone(),
            capability: "beta.cap".to_string(),
        };
        backend
            .record_invocation(
                &beta_key,
                &ev("beta-evt"),
                at_secs(0),
                Duration::from_secs(60),
            )
            .await
            .expect("ok");

        for i in 0..=MAX_KEYS_PER_TENANT {
            let key = InvocationKey {
                hook_id: hook_id(),
                tenant_id: alpha.clone(),
                capability: format!("alpha.cap.{i}"),
            };
            let _ = backend
                .record_invocation(
                    &key,
                    &ev(&format!("alpha-e{i}")),
                    at_millis(i as i64 + 1),
                    Duration::from_secs(60),
                )
                .await
                .expect("ok");
        }

        let map = backend.invocation_history.lock().expect("ok");
        assert!(
            map.contains_key(&beta_key),
            "noisy tenant α must not evict quiet tenant β's bucket"
        );
        let alpha_count = map.keys().filter(|k| k.tenant_id == alpha).count();
        assert_eq!(
            alpha_count, MAX_KEYS_PER_TENANT,
            "noisy tenant α must be capped at its per-tenant quota"
        );
    }

    /// henrypark133 MED on PR #3635 5-19 review: `evict_older_than` drops
    /// entries strictly older than the cutoff and removes empty buckets.
    #[tokio::test]
    async fn evict_older_than_drops_expired_entries_and_empty_buckets() {
        let backend = InMemoryPredicateStateBackend::new();
        let window = Duration::from_secs(3600);

        let stale_key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.stale".to_string(),
        };
        let live_key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.live".to_string(),
        };
        backend
            .record_invocation(&stale_key, &ev("s1"), at_secs(0), window)
            .await
            .expect("ok");
        backend
            .record_invocation(&stale_key, &ev("s2"), at_secs(1), window)
            .await
            .expect("ok");
        backend
            .record_invocation(&live_key, &ev("l1"), at_secs(120), window)
            .await
            .expect("ok");

        let cutoff = at_secs(60);
        let dropped = backend.evict_older_than(cutoff).await.expect("evict ok");
        assert_eq!(dropped, 2, "two stale entries must be dropped");

        let map = backend.invocation_history.lock().expect("ok");
        assert!(
            !map.contains_key(&stale_key),
            "fully-stale bucket must be removed after reaper drains it"
        );
        assert!(
            map.contains_key(&live_key),
            "live bucket must survive the reaper pass"
        );
    }

    /// henrypark133 HIGH on PR #3635 5-19 review: NumericSum sums must be
    /// O(1) via an incrementally-maintained `running_sum`; the invariant
    /// is that `running_sum` matches the deque sum after every push/pop.
    #[tokio::test]
    async fn numeric_sum_running_sum_matches_deque_after_trim_and_replay() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = ValueKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.spend".to_string(),
            field: "amount".to_string(),
        };
        let window = Duration::from_secs(60);

        let s1 = backend
            .record_value(&key, &ev("v1"), at_secs(0), Decimal::from(10), window)
            .await
            .expect("ok");
        assert_eq!(s1, Decimal::from(10));

        let s2 = backend
            .record_value(&key, &ev("v2"), at_secs(1), Decimal::from(25), window)
            .await
            .expect("ok");
        assert_eq!(s2, Decimal::from(35));

        // Replay v1: dedup, sum unchanged.
        let s_replay = backend
            .record_value(&key, &ev("v1"), at_secs(2), Decimal::from(10), window)
            .await
            .expect("ok");
        assert_eq!(s_replay, Decimal::from(35), "replay must not double-count");

        // Advance past window so both prior entries trim.
        let s_after_trim = backend
            .record_value(&key, &ev("v3"), at_secs(120), Decimal::from(7), window)
            .await
            .expect("ok");
        assert_eq!(
            s_after_trim,
            Decimal::from(7),
            "running_sum must subtract trimmed entries; sum is just v3"
        );
    }

    /// Threat-model finding D5: per-key sample cap drops oldest samples
    /// first so the bucket never grows past `MAX_SAMPLES_PER_KEY` under
    /// attacker-triggered hot capabilities.
    #[tokio::test]
    async fn record_invocation_caps_samples_per_key_under_attacker_pressure() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.hot".to_string(),
        };
        let window = Duration::from_secs(3600);
        let overflow = MAX_SAMPLES_PER_KEY + 64;
        for i in 0..overflow {
            let _ = backend
                .record_invocation(&key, &ev(&format!("evt-{i}")), at_millis(i as i64), window)
                .await
                .expect("ok");
        }
        let history = backend.invocation_history.lock().expect("lock");
        let bucket = history.get(&key).expect("bucket retained");
        assert!(
            bucket.entries.len() <= MAX_SAMPLES_PER_KEY,
            "bucket must not grow past per-key cap; got {}",
            bucket.entries.len()
        );
        assert_eq!(
            bucket.entries.len(),
            MAX_SAMPLES_PER_KEY,
            "drop-oldest must keep the bucket pinned at the cap under sustained pressure"
        );
        assert_eq!(
            bucket.dedup_ids.len(),
            bucket.entries.len(),
            "dedup_ids must mirror entries after cap-driven eviction"
        );
    }

    /// Threat-model finding D5 for the NumericSum path: `running_sum`
    /// invariant must survive cap-driven eviction.
    #[tokio::test]
    async fn record_value_evicts_oldest_keeping_running_sum_consistent() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = ValueKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.spend".to_string(),
            field: "amount".to_string(),
        };
        let window = Duration::from_secs(3600);
        let overflow = MAX_SAMPLES_PER_KEY + 32;
        let value = Decimal::from(3);
        for i in 0..overflow {
            let _ = backend
                .record_value(
                    &key,
                    &ev(&format!("v-{i}")),
                    at_millis(i as i64),
                    value,
                    window,
                )
                .await
                .expect("ok");
        }
        let history = backend.value_history.lock().expect("lock");
        let bucket = history.get(&key).expect("bucket retained");
        assert!(
            bucket.entries.len() <= MAX_SAMPLES_PER_KEY,
            "bucket must not grow past per-key cap; got {}",
            bucket.entries.len()
        );
        assert_eq!(bucket.entries.len(), MAX_SAMPLES_PER_KEY);
        let deque_sum: Decimal = bucket.entries.iter().map(|(_, v, _)| *v).sum();
        assert_eq!(
            bucket.running_sum, deque_sum,
            "running_sum must stay in sync with deque content after cap-driven eviction"
        );
        assert_eq!(
            bucket.running_sum,
            Decimal::from(MAX_SAMPLES_PER_KEY as u64) * value,
            "with constant value inserts, post-cap sum = cap * value"
        );
        assert_eq!(
            bucket.dedup_ids.len(),
            bucket.entries.len(),
            "dedup_ids must mirror entries after cap-driven eviction"
        );
    }

    /// henrypark133 missing-coverage on PR #3635: prove the atomic
    /// record-and-read contract holds under concurrent writers. N tasks
    /// each call `record_invocation` once with a distinct `event_id`; the
    /// final observed count must equal N (no lost-update race).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn record_invocation_is_atomic_under_concurrent_writers() {
        use std::sync::Arc as StdArc;
        let backend = StdArc::new(InMemoryPredicateStateBackend::new());
        let key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.concurrent".to_string(),
        };
        let now = at_secs(0);
        const N: usize = 32;
        let mut handles = Vec::with_capacity(N);
        for i in 0..N {
            let backend = StdArc::clone(&backend);
            let key = key.clone();
            let ev = ev(&format!("event-{i}"));
            handles.push(tokio::spawn(async move {
                backend
                    .record_invocation(&key, &ev, now, Duration::from_secs(60))
                    .await
                    .expect("ok")
            }));
        }
        for h in handles {
            h.await.expect("task joined");
        }
        // Re-read via a no-op record with a duplicate id to observe the
        // final count without inserting a new entry.
        let final_count = backend
            .record_invocation(&key, &ev("event-0"), now, Duration::from_secs(60))
            .await
            .expect("ok");
        assert_eq!(
            final_count as usize, N,
            "N concurrent distinct-id writes must each be counted exactly once \
             (atomic record-and-read contract)"
        );
    }
}
