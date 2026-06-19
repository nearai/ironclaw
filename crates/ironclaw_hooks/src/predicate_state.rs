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

/// Maximum byte length of a [`PredicateEventId`]. The id is stored verbatim
/// in the durable backends' `event_id` TEXT column (no DB-level length
/// constraint), so an attacker who can drive `event_id` through a
/// `record_invocation` / `record_value` path could otherwise insert
/// arbitrarily large strings — up to PostgreSQL's 1 GiB row limit — into
/// durable storage, and with [`MAX_SAMPLES_PER_KEY`] in-window samples per
/// key the multiplier is large (henrypark133 MEDIUM, PR #3937). The
/// [`WindowOverflow`] error also embeds a key label in its message, so an
/// unbounded id amplifies into error-message memory.
///
/// `512` is ~4× a 64-char blake3 hex digest (the canonical synth shape) and
/// matches [`crate::manifest::MAX_MANIFEST_REASON_BYTES`], the other
/// operator-facing byte cap in this crate. Measured in bytes (not chars) so a
/// multibyte-UTF-8 id can't slip past a char count.
///
/// [`WindowOverflow`]: PredicateBackendError::WindowOverflow
pub const MAX_EVENT_ID_LEN: usize = 512;

/// Format-validation error for [`PredicateEventId::new`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PredicateEventIdError {
    #[error("predicate event id must not be empty")]
    Empty,
    #[error("predicate event id must not contain NUL bytes")]
    ContainsNul,
    #[error("predicate event id is {len} bytes, exceeding the maximum of {max}")]
    TooLong { len: usize, max: usize },
}

impl PredicateEventId {
    /// Construct from any string-like value, validating non-empty,
    /// within [`MAX_EVENT_ID_LEN`] bytes, and NUL-free at the type
    /// boundary. Returns
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
        if value.len() > MAX_EVENT_ID_LEN {
            return Err(PredicateEventIdError::TooLong {
                len: value.len(),
                max: MAX_EVENT_ID_LEN,
            });
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

    /// The per-key sliding window hit its sample cap
    /// ([`MAX_SAMPLES_PER_KEY`]) and a new in-window sample could not be
    /// recorded without dropping an existing one. Returned **fail-closed**
    /// (codex review on PR #3635): silently evicting the oldest sample to
    /// make room weakened cap enforcement (a count could never exceed the
    /// cap) and broke replay refusal (the evicted sample's id left the
    /// dedup set while still logically in-window). The evaluator treats
    /// this as a restrictive DENY/PauseApproval rather than a silent Allow.
    ///
    /// For `InvocationCount` predicates this is unreachable in practice:
    /// manifest validation rejects any `max` above the cap, so a
    /// well-formed cap denies (count > max) before the window can overflow.
    /// For `NumericSum` predicates there is no per-sample count threshold
    /// to bound the window, so this is the backpressure signal.
    #[error(
        "predicate sliding window for `{key}` exceeded the per-key sample cap \
         of {cap}; failing closed"
    )]
    WindowOverflow { key: String, cap: usize },
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
    ///
    /// # Operator requirement (durable backends)
    ///
    /// Durable backends MUST have this scheduled periodically (typically at the
    /// slowest configured window). The per-tenant LRU quota counts ALL stored
    /// rows for a tenant, including expired-but-unreaped rows from idle keys;
    /// without a reaper, short-window workloads can appear at
    /// `MAX_KEYS_PER_TENANT` and trigger LRU eviction (advancing
    /// `evictions_observed`) below their active key count. The `record_*` path
    /// only trims the current key's window, never sibling keys, so this method
    /// is the only thing that reclaims idle expired rows. See
    /// `docs/successors/03-persistent-counter.md` (Reaper requirement).
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

/// Per-tenant ceiling on distinct keys held across both history maps.
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
///
/// # Canonical cross-backend cutoff (do not reimplement)
///
/// This is the **single source of truth** for the sliding-window cutoff rule
/// (`occurred_at < cutoff` is trimmed; an entry exactly at `cutoff` is
/// retained). Durable backends (libSQL, Postgres) MUST derive their cutoff from
/// this function rather than reimplementing the `Duration → wall-clock`
/// conversion, so the overflow/boundary behaviour cannot drift per backend. A
/// libSQL backend storing epoch-millis converts the result with
/// [`DateTime::timestamp_millis`]; the `i64::MAX`-saturating-then-`saturating_sub`
/// shortcut a backend might write independently is **not** equivalent — on an
/// oversized window it trims nothing, whereas this canonical rule trims to
/// `now`. The cross-backend parity suite (#3937) covers this boundary.
pub fn window_cutoff(now: DateTime<Utc>, window: Duration) -> DateTime<Utc> {
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
        let mut invocation_history = match self.invocation_history.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut value_history = match self.value_history.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !invocation_history.contains_key(key) {
            // Per-tenant quota: if this tenant already holds the cap,
            // counting BOTH history maps, evict their oldest-front bucket
            // FIRST (not a global LRU pass) so noisy tenants can't push quiet
            // tenants out.
            let tenant_count = tenant_invocation_key_count(&invocation_history, &key.tenant_id)
                + tenant_value_key_count(&value_history, &key.tenant_id);
            if tenant_count >= MAX_KEYS_PER_TENANT {
                evict_lru_for_tenant(
                    &mut invocation_history,
                    &mut value_history,
                    &key.tenant_id,
                    &self.evictions,
                );
            } else if invocation_history.len() >= MAX_HISTORY_KEYS {
                evict_lru_invocation(&mut invocation_history, &self.evictions);
            }
        }
        let bucket = invocation_history.entry(key.clone()).or_default();
        // Trim entries outside the window using a wall-clock cutoff. With
        // the `DateTime<Utc>` clock, `window_cutoff` computes `now - window`
        // via saturating `chrono` arithmetic, so the `Instant::checked_sub`
        // underflow that collapsed long windows to "only now" (codex review
        // on PR #3635, Bug 2) cannot arise — year-2026 minus any realistic
        // window stays well above `DateTime::MIN`. The strict `<` keeps an
        // entry exactly at the cutoff in-window. Trimming via `pop_front`
        // keeps the `dedup_ids` set in sync.
        let cutoff = window_cutoff(now, window);
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
            // Per-key sample cap: FAIL CLOSED on overflow (codex review on
            // PR #3635, Bug 1). The previous implementation dropped the
            // oldest sample to make room, which silently weakened cap
            // enforcement (the count could never exceed the cap, so an
            // `InvocationCount { max >= cap }` never fired) and broke replay
            // refusal (the evicted id left `dedup_ids` while still in-window).
            // We now reject instead. Manifest validation rejects count
            // thresholds above the cap, so a well-formed `InvocationCount`
            // denies (count > max) before this can trip; reaching here means
            // genuine backpressure that must not silently pass.
            if bucket.entries.len() >= MAX_SAMPLES_PER_KEY {
                // Drop the bucket if the trim above emptied it, so an
                // overflow-on-a-stale-key doesn't leave a zombie bucket.
                if bucket.entries.is_empty() {
                    invocation_history.remove(key);
                }
                return Err(PredicateBackendError::WindowOverflow {
                    key: format!("{}/{}", key.tenant_id.as_str(), key.capability),
                    cap: MAX_SAMPLES_PER_KEY,
                });
            }
            bucket.push_back(now, event_id.clone());
        }
        let count = bucket.entries.len() as u32;
        // Drop empty buckets so they can't become zombie LRU-skip keys
        // (codex P1 #2). A bucket gets here empty only if the trim above
        // removed every entry AND `duplicate` was true (so we didn't add
        // a new one).
        if bucket.entries.is_empty() {
            invocation_history.remove(key);
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
        let mut invocation_history = match self.invocation_history.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut value_history = match self.value_history.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !value_history.contains_key(key) {
            let tenant_count = tenant_invocation_key_count(&invocation_history, &key.tenant_id)
                + tenant_value_key_count(&value_history, &key.tenant_id);
            if tenant_count >= MAX_KEYS_PER_TENANT {
                evict_lru_for_tenant(
                    &mut invocation_history,
                    &mut value_history,
                    &key.tenant_id,
                    &self.evictions,
                );
            } else if value_history.len() >= MAX_HISTORY_KEYS {
                evict_lru_value(&mut value_history, &self.evictions);
            }
        }
        let bucket = value_history.entry(key.clone()).or_default();
        // Wall-clock cutoff trim — see `record_invocation` for why the
        // `DateTime<Utc>` clock removes the Bug 2 underflow hazard.
        let cutoff = window_cutoff(now, window);
        while let Some((ts, _, _)) = bucket.entries.front() {
            if *ts < cutoff {
                bucket.pop_front();
            } else {
                break;
            }
        }
        if !bucket.dedup_ids.contains(event_id) {
            // Per-key sample cap: FAIL CLOSED on overflow (codex review on
            // PR #3635, Bug 1). Unlike `InvocationCount`, `NumericSum` has
            // no per-sample count threshold to bound the window at
            // validation time, so the per-key cap is the only sample bound.
            // The old implementation dropped oldest samples to make room,
            // which silently UNDERCOUNTED the sum (in-window values were
            // discarded) — a fail-OPEN weakening of the value cap. We now
            // reject so the evaluator can deny.
            if bucket.entries.len() >= MAX_SAMPLES_PER_KEY {
                if bucket.entries.is_empty() {
                    value_history.remove(key);
                }
                return Err(PredicateBackendError::WindowOverflow {
                    key: format!(
                        "{}/{}#{}",
                        key.tenant_id.as_str(),
                        key.capability,
                        key.field
                    ),
                    cap: MAX_SAMPLES_PER_KEY,
                });
            }
            bucket.push_back(now, value, event_id.clone());
        }
        // O(1): the bucket maintains `running_sum` incrementally on
        // push/pop, so we don't re-walk the deque on every call
        // (henrypark133 HIGH on PR #3635 5-19 review). Snapshot before
        // the potential empty-bucket removal below.
        let sum = bucket.running_sum;
        if bucket.entries.is_empty() {
            value_history.remove(key);
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

fn tenant_invocation_key_count(
    history: &HashMap<InvocationKey, InvocationBucket>,
    tenant_id: &TenantId,
) -> usize {
    history
        .keys()
        .filter(|key| key.tenant_id == *tenant_id)
        .count()
}

fn tenant_value_key_count(history: &HashMap<ValueKey, ValueBucket>, tenant_id: &TenantId) -> usize {
    history
        .keys()
        .filter(|key| key.tenant_id == *tenant_id)
        .count()
}

fn lru_invocation_candidate_for_tenant(
    history: &HashMap<InvocationKey, InvocationBucket>,
    tenant_id: &TenantId,
) -> Option<(InvocationKey, Option<DateTime<Utc>>)> {
    history
        .iter()
        .filter(|(key, _)| key.tenant_id == *tenant_id)
        .map(|(key, bucket)| (key.clone(), bucket.entries.front().map(|(ts, _)| *ts)))
        .min_by_key(|(_, oldest)| *oldest)
}

fn lru_value_candidate_for_tenant(
    history: &HashMap<ValueKey, ValueBucket>,
    tenant_id: &TenantId,
) -> Option<(ValueKey, Option<DateTime<Utc>>)> {
    history
        .iter()
        .filter(|(key, _)| key.tenant_id == *tenant_id)
        .map(|(key, bucket)| (key.clone(), bucket.entries.front().map(|(ts, _, _)| *ts)))
        .min_by_key(|(_, oldest)| *oldest)
}

/// Tenant-scoped aggregate variant: evict the oldest-front bucket BELONGING TO
/// `tenant_id` across BOTH history maps. Used when a single tenant hits
/// [`MAX_KEYS_PER_TENANT`] so the eviction stays within that tenant's combined
/// footprint and cannot reach into another tenant's buckets.
fn evict_lru_for_tenant(
    history: &mut HashMap<InvocationKey, InvocationBucket>,
    value_history: &mut HashMap<ValueKey, ValueBucket>,
    tenant_id: &TenantId,
    evictions: &AtomicU64,
) {
    let invocation_victim = lru_invocation_candidate_for_tenant(history, tenant_id);
    let value_victim = lru_value_candidate_for_tenant(value_history, tenant_id);

    match (invocation_victim, value_victim) {
        (Some((invocation_key, invocation_ts)), Some((value_key, value_ts))) => {
            if invocation_ts <= value_ts {
                history.remove(&invocation_key);
            } else {
                value_history.remove(&value_key);
            }
            evictions.fetch_add(1, AtomicOrdering::Relaxed);
        }
        (Some((invocation_key, _)), None) => {
            history.remove(&invocation_key);
            evictions.fetch_add(1, AtomicOrdering::Relaxed);
        }
        (None, Some((value_key, _))) => {
            value_history.remove(&value_key);
            evictions.fetch_add(1, AtomicOrdering::Relaxed);
        }
        (None, None) => {}
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

    /// Contract: [`MAX_KEYS_PER_TENANT`] is a tenant-wide aggregate quota
    /// across invocation and value histories, not one independent quota per
    /// map. Filling a tenant to the cap with invocation keys, then adding a
    /// value key for the same tenant, must evict one of that tenant's existing
    /// keys and advance the eviction counter.
    pub async fn tenant_key_quota_spans_invocation_and_value_maps<B, F>(factory: F)
    where
        B: PredicateStateBackend,
        F: Fn() -> B,
    {
        let backend = factory();
        let tenant = "alpha";
        let window = Duration::from_secs(86_400);
        for i in 0..MAX_KEYS_PER_TENANT {
            let key = inv_key(tenant, &format!("cap.inv.{i}"));
            backend
                .record_invocation(&key, &ev(&format!("inv-{i}")), at(i as i64), window)
                .await
                .expect("invocation insert under tenant cap succeeds");
        }
        let before_evictions = backend.evictions_observed();

        let value_key = val_key(tenant, "cap.value", "amount");
        let sum = backend
            .record_value(
                &value_key,
                &ev("value-over-cap"),
                at(MAX_KEYS_PER_TENANT as i64 + 1),
                Decimal::from(1),
                window,
            )
            .await
            .expect("value insert should evict within the same tenant, not fail");

        assert_eq!(sum, Decimal::from(1));
        assert!(
            backend.evictions_observed() > before_evictions,
            "adding a value key when invocation keys already fill the tenant \
             quota must trigger aggregate per-tenant eviction"
        );
    }

    /// Contract: the per-key sample cap is FAIL CLOSED. Filling a single
    /// key past [`MAX_SAMPLES_PER_KEY`] with distinct in-window event ids
    /// must return [`PredicateBackendError::WindowOverflow`] rather than
    /// silently evicting the oldest sample (PR #3635 followup / #3929). The
    /// evaluator maps this `Err` to the restrictive `on_exceeded` action, so
    /// overflow surfaces as DENY, never a silent Allow. Every backend —
    /// in-memory and durable — must honor this by construction.
    pub async fn record_invocation_overflow_is_fail_closed<B, F>(factory: F)
    where
        B: PredicateStateBackend,
        F: Fn() -> B,
    {
        let backend = factory();
        let key = inv_key("alpha", "cap.hot");
        let window = Duration::from_secs(3600);
        // Space samples by 1ms so all `MAX_SAMPLES_PER_KEY` stay inside the
        // 1h window (seconds-spacing would push the earliest out of window
        // before the cap is reached).
        let at_ms = |ms: i64| base() + chrono::Duration::milliseconds(ms);
        // Fill exactly to the cap with distinct in-window ids — all succeed.
        for i in 0..MAX_SAMPLES_PER_KEY {
            let count = backend
                .record_invocation(&key, &ev(&format!("e-{i}")), at_ms(i as i64), window)
                .await
                .expect("inserts up to the cap succeed");
            assert_eq!(count as usize, i + 1);
        }
        // The next distinct in-window id must fail closed, not silent-evict.
        let result = backend
            .record_invocation(
                &key,
                &ev("e-overflow"),
                at_ms(MAX_SAMPLES_PER_KEY as i64),
                window,
            )
            .await;
        assert!(
            matches!(result, Err(PredicateBackendError::WindowOverflow { .. })),
            "hitting the per-key cap must fail closed, got {result:?}"
        );
        // A replay of an in-window id at the cap dedups to a no-op (returns
        // the unchanged count) rather than overflowing — replay refusal must
        // survive the cap boundary.
        let replay = backend
            .record_invocation(
                &key,
                &ev("e-0"),
                at_ms(MAX_SAMPLES_PER_KEY as i64 + 1),
                window,
            )
            .await
            .expect("replay of an in-window id must dedup, not overflow");
        assert_eq!(
            replay as usize, MAX_SAMPLES_PER_KEY,
            "replay at the cap is a no-op against the count"
        );
    }

    /// Contract: `evict_older_than` reaps rows STRICTLY older than the cutoff
    /// (`occurred_at < cutoff`, not `<=`) across BOTH the invocation and value
    /// tables, and the returned count equals the number of rows actually
    /// deleted. A backend that reaped with `<=` would also drop the row at the
    /// exact cutoff; a backend that reaped only one table would leave the other
    /// unreaped — both are caught here.
    pub async fn evict_older_than_reaps_strictly_older_rows<B, F>(factory: F)
    where
        B: PredicateStateBackend,
        F: Fn() -> B,
    {
        let backend = factory();
        let inv = inv_key("alpha", "cap.reap");
        let val = val_key("alpha", "cap.reap", "amount");
        // A window large enough that no record_* call trims a sibling — we want
        // evict_older_than to be the only thing that removes rows.
        let window = Duration::from_secs(86_400);
        // Seed t0 (strictly before cutoff), t10 (exactly at cutoff), t20 (after)
        // in BOTH tables.
        for (i, secs) in [0i64, 10, 20].iter().enumerate() {
            backend
                .record_invocation(&inv, &ev(&format!("i{i}")), at(*secs), window)
                .await
                .expect("ok");
            backend
                .record_value(
                    &val,
                    &ev(&format!("v{i}")),
                    at(*secs),
                    Decimal::from(10),
                    window,
                )
                .await
                .expect("ok");
        }

        // Reap everything strictly older than t10. The t0 row in each table is
        // strictly older and must go; the t10 row is exactly at the cutoff and
        // must be retained (< vs <=); t20 survives. Two rows total (one per
        // table) are deleted.
        let dropped = backend.evict_older_than(at(10)).await.expect("evict ok");
        assert_eq!(
            dropped, 2,
            "exactly the two strictly-older rows (one per table) are reaped; \
             the exact-cutoff rows are retained"
        );

        // Observe surviving rows via a probe whose own window cutoff sits before
        // t10, so the probe never trims a survivor. The invocation count and the
        // value sum after the probe expose how many rows the reaper left behind.
        let probe_window = Duration::from_secs(86_400);
        let inv_count = backend
            .record_invocation(&inv, &ev("probe-i"), at(20), probe_window)
            .await
            .expect("ok");
        assert_eq!(
            inv_count, 3,
            "invocation table keeps the t10 (exact-cutoff) and t20 rows; \
             a <= reaper would have dropped t10 and left a count of 2"
        );
        let val_sum = backend
            .record_value(
                &val,
                &ev("probe-v"),
                at(20),
                Decimal::from(10),
                probe_window,
            )
            .await
            .expect("ok");
        assert_eq!(
            val_sum,
            Decimal::from(30),
            "value table keeps the t10 (exact-cutoff) and t20 rows summing 20, \
             plus the 10 probe; a reaper that skipped the value table would sum 40"
        );
    }

    /// Contract: the row whose `occurred_at` equals the cutoff is RETAINED by
    /// `evict_older_than`. Pinpoints the `<` vs `<=` boundary in isolation:
    /// seed a single row at the cutoff, reap at that exact instant, and assert
    /// nothing was dropped and the row is still observable.
    pub async fn evict_older_than_retains_entry_at_exact_cutoff<B, F>(factory: F)
    where
        B: PredicateStateBackend,
        F: Fn() -> B,
    {
        let backend = factory();
        let inv = inv_key("alpha", "cap.cutoff");
        let val = val_key("alpha", "cap.cutoff", "amount");
        let window = Duration::from_secs(86_400);
        backend
            .record_invocation(&inv, &ev("i0"), at(10), window)
            .await
            .expect("ok");
        backend
            .record_value(&val, &ev("v0"), at(10), Decimal::from(10), window)
            .await
            .expect("ok");

        let dropped = backend.evict_older_than(at(10)).await.expect("evict ok");
        assert_eq!(
            dropped, 0,
            "the row exactly at the cutoff is retained (< cutoff, not <=)"
        );

        let probe_window = Duration::from_secs(86_400);
        let inv_count = backend
            .record_invocation(&inv, &ev("probe-i"), at(10), probe_window)
            .await
            .expect("ok");
        assert_eq!(
            inv_count, 2,
            "exact-cutoff invocation row survived the reaper"
        );
        let val_sum = backend
            .record_value(
                &val,
                &ev("probe-v"),
                at(10),
                Decimal::from(10),
                probe_window,
            )
            .await
            .expect("ok");
        assert_eq!(
            val_sum,
            Decimal::from(20),
            "exact-cutoff value row survived the reaper"
        );
    }

    /// The **single canonical inventory** of `PredicateStateBackend` contract
    /// cases. Every shared invariant is listed here exactly once, by the
    /// `pub async fn` name in this module. Both the default-harness wiring
    /// ([`predicate_backend_contract_test!`]) and any out-of-crate custom
    /// runner (e.g. the libSQL serial runner, which cannot use the default
    /// libtest harness) drive their case lists from this macro, so a contract
    /// added here is automatically run against every backend — there is no
    /// hand-maintained second list to drift out of sync.
    ///
    /// `$emit` is a `macro_rules!`-style callback macro the caller supplies; it
    /// is invoked once per case as `$emit!([$($ctx)*] case_name);` where
    /// `case_name` is the identifier of the `contract::` function and `$ctx` is
    /// arbitrary caller context (e.g. a factory expression) threaded through
    /// unchanged. The caller decides what each expansion becomes (a
    /// `#[tokio::test]` fn, a serial-runner line, …).
    #[macro_export]
    macro_rules! predicate_backend_contract_cases {
        ($emit:ident, $($ctx:tt)*) => {
            $emit!([$($ctx)*] invocation_counts_within_window);
            $emit!([$($ctx)*] invocation_trims_outside_window);
            $emit!([$($ctx)*] value_sums_within_window);
            $emit!([$($ctx)*] tenant_isolation);
            $emit!([$($ctx)*] duplicate_event_id_is_noop_for_invocations);
            $emit!([$($ctx)*] duplicate_event_id_is_noop_for_values);
            $emit!([$($ctx)*] invocation_retains_entry_at_exact_window_cutoff);
            $emit!([$($ctx)*] event_id_dedup_isolated_across_maps);
            $emit!([$($ctx)*] tenant_key_quota_spans_invocation_and_value_maps);
            $emit!([$($ctx)*] record_invocation_overflow_is_fail_closed);
            $emit!([$($ctx)*] evict_older_than_reaps_strictly_older_rows);
            $emit!([$($ctx)*] evict_older_than_retains_entry_at_exact_cutoff);
        };
    }

    /// Run every contract against `factory` under the default libtest harness.
    /// Per-impl test files invoke this via [`predicate_backend_contract_test!`].
    /// The case inventory is the canonical [`predicate_backend_contract_cases!`]
    /// list — this macro only decides that each case becomes a
    /// `#[tokio::test]`.
    #[macro_export]
    macro_rules! predicate_backend_contract_test {
        ($label:ident, $factory:expr) => {
            mod $label {
                macro_rules! emit_case {
                    ([$factory_expr:expr] $case:ident) => {
                        #[tokio::test]
                        async fn $case() {
                            $crate::predicate_state::contract::$case($factory_expr).await;
                        }
                    };
                }
                $crate::predicate_backend_contract_cases!(emit_case, $factory);
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

    /// henrypark133 MEDIUM on PR #3937: `PredicateEventId::new` must cap
    /// length so an attacker-driven `event_id` can't insert arbitrarily large
    /// strings into the durable `event_id` TEXT column. Boundary: exactly
    /// `MAX_EVENT_ID_LEN` bytes is accepted; one byte over is rejected with
    /// the typed `TooLong` variant carrying the observed length and cap.
    #[test]
    fn predicate_event_id_accepts_max_length() {
        let at_cap = "a".repeat(MAX_EVENT_ID_LEN);
        assert!(
            PredicateEventId::new(at_cap.clone()).is_ok(),
            "exactly MAX_EVENT_ID_LEN bytes is in-bounds"
        );
    }

    #[test]
    fn predicate_event_id_rejects_too_long() {
        let over = "a".repeat(MAX_EVENT_ID_LEN + 1);
        assert_eq!(
            PredicateEventId::new(over),
            Err(PredicateEventIdError::TooLong {
                len: MAX_EVENT_ID_LEN + 1,
                max: MAX_EVENT_ID_LEN,
            })
        );
    }

    /// Length is measured in BYTES, not chars: a multibyte-UTF-8 id whose
    /// char count is under the cap but whose byte length is over must still be
    /// rejected (the durable column is sized in bytes).
    #[test]
    fn predicate_event_id_length_cap_is_measured_in_bytes() {
        // '𝄞' (U+1D11E) is 4 bytes. (MAX/4 + 1) of them exceeds the byte cap
        // while the char count stays well under it.
        let multibyte = "𝄞".repeat(MAX_EVENT_ID_LEN / 4 + 1);
        assert!(multibyte.chars().count() <= MAX_EVENT_ID_LEN);
        assert!(
            multibyte.len() > MAX_EVENT_ID_LEN,
            "fixture must exceed the byte cap"
        );
        assert!(matches!(
            PredicateEventId::new(multibyte),
            Err(PredicateEventIdError::TooLong { .. })
        ));
    }

    /// henrypark133 MEDIUM on PR #3937: pin the `Err(_) => now` overflow branch
    /// of the canonical `window_cutoff`. A `std::time::Duration` larger than
    /// `chrono::Duration::MAX` (~292 million years) overflows
    /// `chrono::Duration::from_std`; the canonical rule trims to `now` (NOT
    /// nothing — the divergence the rustdoc warns against). No other test
    /// exercises this branch, so a refactor to a `saturating_sub`/`unwrap_or`
    /// shortcut would silently regress without it.
    #[test]
    fn window_cutoff_oversized_window_trims_to_now() {
        let now = base();
        assert_eq!(
            window_cutoff(now, Duration::MAX),
            now,
            "an oversized window overflows from_std and trims to now, not nothing"
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

    /// henrypark133 missing-coverage on PR #3635: exact-cutoff boundary
    /// behavior. The trim condition is `front_ts < cutoff`, so an entry
    /// recorded at exactly `cutoff` is RETAINED, not trimmed. Pin that
    /// boundary so a future refactor to `<=` would fail loud.
    #[tokio::test]
    async fn in_memory_invocation_retains_entry_at_exact_window_cutoff() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.boundary".to_string(),
        };
        let window = Duration::from_secs(60);
        let t0 = at_secs(0);
        // Record at t0.
        let _ = backend
            .record_invocation(&key, &ev("e-at-t0"), t0, window)
            .await
            .expect("ok");
        // At t0 + window, the cutoff is exactly t0; t0 is NOT strictly less
        // than t0, so the original entry must still be in-window.
        let count_at_exact_cutoff = backend
            .record_invocation(&key, &ev("e-at-boundary"), at_secs(60), window)
            .await
            .expect("ok");
        assert_eq!(
            count_at_exact_cutoff, 2,
            "entry whose timestamp equals the cutoff is retained (< cutoff trim, not <=)"
        );
        // One millisecond past the cutoff, the original entry IS trimmed.
        let count_just_past = backend
            .record_invocation(&key, &ev("e-just-past"), at_millis(60_001), window)
            .await
            .expect("ok");
        // After the just-past call: e-at-t0 is now strictly older than
        // cutoff and gets trimmed; e-at-boundary and e-just-past remain.
        assert_eq!(
            count_just_past, 2,
            "entry strictly older than cutoff is trimmed"
        );
    }

    /// henrypark133 missing-coverage on PR #3635: cross-type event-id
    /// isolation. The same `event_id` used in BOTH
    /// `record_invocation` and `record_value` must NOT collide — the two
    /// maps key on disjoint types (`InvocationKey` vs `ValueKey`), so
    /// dedup in one must not suppress the other.
    #[tokio::test]
    async fn event_id_dedup_is_isolated_across_invocation_and_value_maps() {
        let backend = InMemoryPredicateStateBackend::new();
        let inv_key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.cross".to_string(),
        };
        let val_key = ValueKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.cross".to_string(),
            field: "amount".to_string(),
        };
        let shared = ev("shared-event-id");
        let now = at_secs(0);
        let inv_count = backend
            .record_invocation(&inv_key, &shared, now, Duration::from_secs(60))
            .await
            .expect("ok");
        let val_sum = backend
            .record_value(
                &val_key,
                &shared,
                now,
                Decimal::from(42),
                Duration::from_secs(60),
            )
            .await
            .expect("ok");
        assert_eq!(
            inv_count, 1,
            "invocation count records normally; value map's dedup state must not pre-empt it"
        );
        assert_eq!(
            val_sum,
            Decimal::from(42),
            "value sum records normally; invocation map's dedup state must not pre-empt it"
        );
        // Replay each — within its own map the shared id is now a dup.
        let inv_count_replay = backend
            .record_invocation(&inv_key, &shared, now, Duration::from_secs(60))
            .await
            .expect("ok");
        let val_sum_replay = backend
            .record_value(
                &val_key,
                &shared,
                now,
                Decimal::from(42),
                Duration::from_secs(60),
            )
            .await
            .expect("ok");
        assert_eq!(inv_count_replay, 1, "intra-map dedup still works");
        assert_eq!(
            val_sum_replay,
            Decimal::from(42),
            "intra-map dedup still works"
        );
    }

    /// Threat-model finding **D5** regression: an attacker that triggers a
    /// hot capability with a very large declared window can otherwise force
    /// the backend to retain every invocation in the window, exhausting
    /// memory.
    ///
    /// FAIL-CLOSED CHANGE (PR #3635 followup, codex review): the previous
    /// implementation silently dropped the oldest sample to make room when
    /// the per-key cap was hit. That silently weakened cap enforcement (a
    /// count never exceeded [`MAX_SAMPLES_PER_KEY`], so an
    /// `InvocationCount { max }` with `max >= MAX_SAMPLES_PER_KEY` could
    /// never fire) AND broke replay refusal (the evicted sample's id was
    /// removed from `dedup_ids`). The backend now returns
    /// [`PredicateBackendError::WindowOverflow`] instead, which the
    /// evaluator treats as a fail-closed DENY. Validation rejects count
    /// thresholds above the cap at install time so a well-formed
    /// `InvocationCount` cap never reaches overflow.
    #[tokio::test]
    async fn record_invocation_returns_window_overflow_at_cap_not_silent_drop() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.hot".to_string(),
        };
        let window = Duration::from_secs(3600);
        // Fill exactly to the cap — these must all succeed.
        for i in 0..MAX_SAMPLES_PER_KEY {
            let count = backend
                .record_invocation(&key, &ev(&format!("evt-{i}")), at_millis(i as i64), window)
                .await
                .expect("inserts up to the cap succeed");
            assert_eq!(count as usize, i + 1);
        }
        // The next distinct in-window event would push past the cap. It must
        // fail closed (WindowOverflow), NOT silently evict the oldest sample.
        let result = backend
            .record_invocation(&key, &ev("evt-overflow"), at_secs(1), window)
            .await;
        assert!(
            matches!(result, Err(PredicateBackendError::WindowOverflow { .. })),
            "hitting the per-key cap must fail closed, got {result:?}"
        );
        // The bucket must remain pinned at the cap — no silent eviction.
        let history = backend.invocation_history.lock().expect("lock");
        let bucket = history.get(&key).expect("bucket retained");
        assert_eq!(bucket.entries.len(), MAX_SAMPLES_PER_KEY);
        assert_eq!(bucket.dedup_ids.len(), MAX_SAMPLES_PER_KEY);
    }

    /// Companion to the invocation overflow test for the NumericSum path.
    /// `NumericSum` has no per-sample count bound at validation time (its
    /// threshold is a value sum, not a count), so the per-key cap is the
    /// only sample bound. Hitting it must fail closed with
    /// [`PredicateBackendError::WindowOverflow`] rather than silently
    /// undercounting the sum by evicting in-window samples.
    #[tokio::test]
    async fn numeric_sum_window_overflow_returns_deny_not_silent_allow() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = ValueKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.spend".to_string(),
            field: "amount".to_string(),
        };
        let window = Duration::from_secs(3600);
        let value = Decimal::from(3);
        for i in 0..MAX_SAMPLES_PER_KEY {
            let _ = backend
                .record_value(
                    &key,
                    &ev(&format!("v-{i}")),
                    at_millis(i as i64),
                    value,
                    window,
                )
                .await
                .expect("inserts up to the cap succeed");
        }
        let result = backend
            .record_value(&key, &ev("v-overflow"), at_secs(1), value, window)
            .await;
        assert!(
            matches!(result, Err(PredicateBackendError::WindowOverflow { .. })),
            "NumericSum at the per-key cap must fail closed (overflow), got {result:?}"
        );
        // No silent eviction: the running_sum still reflects exactly the
        // cap-many samples, not an undercount.
        let history = backend.value_history.lock().expect("lock");
        let bucket = history.get(&key).expect("bucket retained");
        assert_eq!(bucket.entries.len(), MAX_SAMPLES_PER_KEY);
        let deque_sum: Decimal = bucket.entries.iter().map(|(_, v, _)| *v).sum();
        assert_eq!(
            bucket.running_sum, deque_sum,
            "running_sum must stay in sync with deque content"
        );
        assert_eq!(
            bucket.running_sum,
            Decimal::from(MAX_SAMPLES_PER_KEY as u64) * value,
        );
    }

    /// codex review (PR #3635 followup): replay refusal must survive cap
    /// pressure. Even when the bucket is saturated at the per-key cap, an
    /// id already recorded within the window must still dedup to a no-op
    /// (and report the unchanged count) rather than being treated as a
    /// fresh event. The dedup-id lifecycle is the logical window, not a
    /// fixed ring that the cap can silently shrink.
    #[tokio::test]
    async fn dedup_id_survives_cap_eviction_within_window() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.replay".to_string(),
        };
        let window = Duration::from_secs(3600);
        // Saturate the bucket to the cap.
        for i in 0..MAX_SAMPLES_PER_KEY {
            backend
                .record_invocation(&key, &ev(&format!("evt-{i}")), at_millis(i as i64), window)
                .await
                .expect("ok");
        }
        // Replaying the FIRST (oldest) in-window id must be a no-op: it is
        // still inside the window, so its presence in dedup_ids must be
        // honored even at the cap boundary. A fresh count is returned, NOT
        // an overflow, because no new sample is added.
        let count = backend
            .record_invocation(&key, &ev("evt-0"), at_secs(2), window)
            .await
            .expect("replay of in-window id must dedup, not overflow");
        assert_eq!(
            count as usize, MAX_SAMPLES_PER_KEY,
            "replay of an in-window id at the cap must be a no-op against the count"
        );
    }

    /// codex review (PR #3635 followup) — Bug 2: a long window must not
    /// collapse to "only now". Under the prior `Instant` clock,
    /// `now.checked_sub(window).unwrap_or(now)` underflowed the monotonic
    /// clock for a 24h window shortly after boot and collapsed the cutoff to
    /// `now`, trimming every prior in-window entry. The `DateTime<Utc>` clock
    /// (PR #3927) computes `now - window` via saturating `chrono` arithmetic
    /// — a wall-clock instant minus 24h cannot underflow — so the cutoff is a
    /// real prior timestamp and in-window entries are retained. This test
    /// pins that intent against the new clock.
    #[tokio::test]
    async fn record_invocation_with_long_window_does_not_zero_cutoff() {
        let backend = InMemoryPredicateStateBackend::new();
        let key = InvocationKey {
            hook_id: hook_id(),
            tenant_id: tenant(),
            capability: "cap.longwindow".to_string(),
        };
        // Two events a few ms apart are both well within a 24h window and
        // must both be counted; the long window must not collapse the cutoff.
        let now = at_secs(0);
        let window = Duration::from_secs(24 * 3600);
        backend
            .record_invocation(&key, &ev("first"), now, window)
            .await
            .expect("ok");
        let count = backend
            .record_invocation(&key, &ev("second"), at_millis(5), window)
            .await
            .expect("ok");
        assert_eq!(
            count, 2,
            "a long window must retain prior in-window entries; the cutoff must not collapse to now"
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
