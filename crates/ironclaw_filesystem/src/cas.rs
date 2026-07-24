//! Shared, mutex-free compare-and-swap (CAS) read-modify-write helper.
//!
//! IronClaw Reborn persistence stores keep their durable state as a single
//! versioned snapshot per scoped key (e.g. `/turns/state.json`). Mutating that
//! snapshot is a read-modify-write: load the current snapshot + its
//! [`RecordVersion`](crate::RecordVersion), compute the next snapshot, then
//! [`put`](RootFilesystem::put) it back with a
//! `CasExpectation::Version` precondition so a concurrent writer that committed
//! in between is detected as a [`FilesystemError::VersionMismatch`] instead of
//! silently clobbered.
//!
//! Historically each store wrapped that loop in a per-record
//! `tokio::sync::Mutex` held across `.await`. Under burst those locks formed a
//! convoy that wedged the runtime (2026-06-24 incident). PR #5142 removed the
//! mutex from `ironclaw_turns` and proved the lock-free pattern: an optimistic
//! CAS-retry loop with bounded retries, jittered exponential backoff, and an
//! overall timeout. [`cas_update`] extracts that **exact** pattern so every
//! store can drop its mutex onto one audited implementation.
//!
//! ## Contract
//!
//! [`cas_update`] never holds a lock across an `.await`. It runs a bounded
//! read-modify-write loop:
//!
//! 1. Read the current versioned snapshot for `path` (decoding it via the
//!    caller's `decode`, or treating an absent record as `None`).
//! 2. Run the caller's `apply` closure, which computes the next snapshot plus a
//!    caller-defined outcome (`T`) or returns the caller's error (`E`).
//! 3. If the snapshot is unchanged (or the caller explicitly returns a no-op),
//!    return the outcome without writing.
//! 4. Otherwise either `put` the encoded snapshot back with the read version as
//!    the CAS precondition (`CasExpectation::Absent` for a first write), or
//!    conditionally delete it when the caller returns [`CasApply::delete`].
//! 5. On [`FilesystemError::VersionMismatch`] re-read and retry, sleeping with
//!    jittered exponential backoff between attempts. A successful conditional
//!    delete is also re-read and re-applied before returning, so a delete +
//!    recreate ABA cycle cannot satisfy the caller's postcondition
//!    prematurely.
//!
//! The whole loop is wrapped in a [`FILESYSTEM_APPLY_TIMEOUT`] timeout so one
//! wedged backend operation consumes only this caller's attempt rather than
//! parking unrelated callers forever. Retries are capped at
//! [`FILESYSTEM_CAS_RETRIES`]; exceeding the cap returns
//! [`CasUpdateError::RetriesExhausted`] rather than looping indefinitely.
//!
//! ## Constants (ported verbatim from the `ironclaw_turns` reference)
//!
//! - [`FILESYSTEM_CAS_RETRIES`] = 32 — mutation-attempt cap under contention.
//! - [`FILESYSTEM_APPLY_TIMEOUT`] = 15s — deadline for the entire loop.
//! - [`FILESYSTEM_CAS_BACKOFF_BASE`] = 2ms — backoff at the first retry.
//! - [`FILESYSTEM_CAS_BACKOFF_MAX`] = 50ms — backoff ceiling.
//!
//! ## Capability gate (fail closed)
//!
//! Before attempting any mutation, [`cas_update`] asserts the backend can honor
//! compare-and-swap. A [`CasApply::delete`] additionally requires the declared
//! [`Capability::Delete`]. There are two layers, because the production
//! composite router cannot answer capabilities without a concrete path (see
//! [`ScopedFilesystem::capabilities`]):
//!
//! - **Pre-flight:** if the backend advertises a *known* capability shape
//!   (anything other than the empty default), and that shape does **not**
//!   include [`TxnCapability::Cas`] (or richer), the helper refuses up front
//!   with [`CasUpdateError::CasUnsupported`]. This catches a misconfigured
//!   byte-only mount before it can blind-overwrite a snapshot.
//! - **Op-time:** an empty/unknown capability shape (the composite router)
//!   defers to the write, where an
//!   [`FilesystemError::Unsupported`] for the attempted versioned write or
//!   conditional delete is mapped to the same
//!   [`CasUpdateError::CasUnsupported`]. Either way the helper fails closed
//!   instead of falling back to an unconditional mutation.
//!
//! ## Error mapping
//!
//! The helper is generic over the caller's error type `E` and never leaks a
//! store-specific type. Helper-level failures surface as [`CasUpdateError`];
//! callers convert them into their own error enum with a single mapper closure
//! (`map_err`). The caller's own `apply` error (`E`) is returned unwrapped via
//! [`CasUpdateError::Apply`], so a store maps the whole space in one place.

use std::future::Future;
use std::time::Duration;

use ironclaw_host_api::{ResourceScope, ScopedPath};

use crate::{
    BackendCapabilities, Capability, CasExpectation, Entry, FilesystemError, FilesystemOperation,
    RootFilesystem, ScopedFilesystem, TxnCapability,
};

/// Bound on the CAS retry loop. Ported from the `ironclaw_turns` reference: the
/// per-key snapshot is written with optimistic CAS instead of an in-process
/// write gate, so bursts of same-key transitions can overlap without parking
/// unrelated callers behind one wedged operation.
pub const FILESYSTEM_CAS_RETRIES: usize = 32;
/// Deadline for the entire read-modify-write loop (including all retries).
pub const FILESYSTEM_APPLY_TIMEOUT: Duration = Duration::from_secs(15);
/// Backoff applied before the first retry; doubles each attempt up to
/// [`FILESYSTEM_CAS_BACKOFF_MAX`].
pub const FILESYSTEM_CAS_BACKOFF_BASE: Duration = Duration::from_millis(2);
/// Ceiling on the exponential backoff between CAS retries.
pub const FILESYSTEM_CAS_BACKOFF_MAX: Duration = Duration::from_millis(50);

/// Outcome of a single `apply` invocation inside [`cas_update`].
///
/// `snapshot` is the next snapshot to persist; `outcome` is whatever the caller
/// wants returned from the whole `cas_update` call on success.
///
/// The caller selects one of three typed mutation outcomes:
///
/// 1. **Write** — return `CasApply::new`. When the returned snapshot equals the
///    existing value that `apply` received, `cas_update` skips the write as a
///    convenience fast-path for callers with a `PartialEq` snapshot.
/// 2. **No-op** — return `CasApply::no_op`. This unconditionally bypasses the
///    mutation regardless of `snapshot`; use it when `current` is `None` and
///    no empty/default record should be created.
/// 3. **Delete** — return `CasApply::delete`. The helper conditionally deletes
///    the version just read, then re-runs `apply` against a fresh read to prove
///    the caller's postcondition across delete + recreate ABA cycles.
pub struct CasApply<S, T> {
    /// The new snapshot to write back. Ignored for persistence when writing is
    /// not selected (i.e., constructed via [`CasApply::no_op`] or
    /// [`CasApply::delete`]).
    pub snapshot: S,
    /// The value to return from [`cas_update`] on success.
    pub outcome: T,
    operation: CasApplyOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CasApplyOperation {
    Write,
    NoOp,
    Delete,
}

impl<S, T> CasApply<S, T> {
    /// Produce a [`CasApply`] that **writes** `snapshot` back to the store.
    ///
    /// All existing callers use this constructor; its signature is unchanged.
    /// If `snapshot` equals the value `apply` was handed (i.e. nothing
    /// changed), `cas_update` still skips the write via the `PartialEq` fast
    /// path — no API change needed for callers that rely on that behavior.
    pub fn new(snapshot: S, outcome: T) -> Self {
        Self {
            snapshot,
            outcome,
            operation: CasApplyOperation::Write,
        }
    }

    /// Produce a [`CasApply`] that **skips** the write unconditionally.
    ///
    /// Use this when the caller wants to signal a no-op for a case the
    /// `PartialEq` fast path cannot cover — primarily when `current` is `None`
    /// (absent record) and the computed snapshot equals the type's default.
    /// `snapshot` is carried in the struct for type-safety but is not
    /// persisted.
    pub fn no_op(snapshot: S, outcome: T) -> Self {
        Self {
            snapshot,
            outcome,
            operation: CasApplyOperation::NoOp,
        }
    }

    /// Produce a [`CasApply`] that conditionally deletes the current snapshot.
    ///
    /// The helper calls [`ScopedFilesystem::delete_if_version`] with the
    /// version read for this `apply` invocation. A lost race is retried through
    /// the same bounded, jittered loop as a versioned write. After a successful
    /// delete (or a concurrent `NotFound`), the helper unconditionally re-reads
    /// and re-runs `apply`; it returns only after the caller reports that the
    /// freshly observed state satisfies its postcondition. This closes the ABA
    /// hole for paths whose version restarts after delete + recreate. A
    /// verification-only invocation does not replace `outcome`: after the
    /// postcondition is established, the helper returns the outcome from the
    /// successful delete invocation.
    ///
    /// `snapshot` is retained for the existing strongly typed result shape but
    /// is not encoded or persisted.
    pub fn delete(snapshot: S, outcome: T) -> Self {
        Self {
            snapshot,
            outcome,
            operation: CasApplyOperation::Delete,
        }
    }
}

/// Failure modes of [`cas_update`].
///
/// Generic over the caller's error type `E` ([`CasUpdateError::Apply`]) and
/// otherwise store-agnostic. Callers map the non-`Apply` variants into their
/// own error enum via the `map_err` argument; the `Apply` variant carries the
/// caller's own error straight through.
#[derive(Debug)]
pub enum CasUpdateError<E> {
    /// The caller's `apply` closure returned an error. Carried through
    /// unchanged so callers don't double-wrap their own error type.
    Apply(E),
    /// The whole read-modify-write loop exceeded [`FILESYSTEM_APPLY_TIMEOUT`].
    Timeout,
    /// [`FILESYSTEM_CAS_RETRIES`] mutation attempts could not establish the
    /// postcondition. The snapshot is under sustained contention from other
    /// writers (or a backend reporting persistent `VersionMismatch`); the
    /// caller should surface a retryable/unavailable error rather than loop
    /// forever.
    RetriesExhausted,
    /// The backend cannot honor compare-and-swap (no
    /// [`TxnCapability::Cas`]) or a requested conditional delete. Surfaced
    /// from the pre-flight capability gate or from an op-time `Unsupported`
    /// mutation. Fail-closed: the helper refuses rather than applying an
    /// unconditional mutation.
    CasUnsupported,
    /// Any other backend or serialization failure during read/decode/encode/
    /// mutation. Carries the underlying [`FilesystemError`] for context.
    Backend(FilesystemError),
}

impl<E: std::fmt::Display> std::fmt::Display for CasUpdateError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Apply(error) => write!(f, "cas_update apply failed: {error}"),
            Self::Timeout => f.write_str("cas_update timed out"),
            Self::RetriesExhausted => f.write_str("cas_update CAS retries exhausted"),
            Self::CasUnsupported => {
                f.write_str("cas_update backend does not support the requested CAS mutation")
            }
            Self::Backend(error) => write!(f, "cas_update backend error: {error}"),
        }
    }
}

impl<E: std::fmt::Display + std::fmt::Debug> std::error::Error for CasUpdateError<E> {}

/// Lock-free, bounded, capability-gated CAS read-modify-mutate over a single
/// scoped snapshot at `path`.
///
/// See the [module docs](self) for the full contract, retry/timeout/backoff
/// constants, and the capability-gate rationale.
///
/// ## Type parameters
///
/// - `S` — the decoded snapshot/record type. `decode` turns the stored body
///   into an `S`; `encode` turns the next `S` into an [`Entry`] to persist.
/// - `T` — the caller's success outcome, returned from `cas_update`.
/// - `E` — the caller's own error type. Returned unwrapped via
///   [`CasUpdateError::Apply`].
///
/// ## Closure shapes (Phase 2 store-migrators: read this)
///
/// - `decode: Fn(&[u8]) -> Result<S, E>` — deserialize a stored snapshot body.
/// - `encode: Fn(&S) -> Result<Entry, E>` — serialize the next snapshot into a
///   versioned [`Entry`] (set `kind`/`content_type` here).
/// - `apply: FnMut(Option<S>) -> Future<Output = Result<CasApply<S, T>, E>>` —
///   receives the current snapshot (`None` when the record is absent / first
///   write) and computes the next snapshot + outcome. **Must be idempotent /
///   re-runnable**: it is re-invoked on every CAS retry against a freshly read
///   snapshot, so it must not mutate external state.
///
///   The mutation signals are:
///   - Return `CasApply::no_op(snapshot, outcome)` to skip the write for any
///     case, including when `current` is `None` (absent record).
///   - Return `CasApply::new(unchanged_snapshot, outcome)` where
///     `unchanged_snapshot` equals what `apply` was handed; `cas_update`
///     detects the equality via `PartialEq` and skips the write as a
///     convenience fast path.
///   - Return `CasApply::delete(snapshot, outcome)` to conditionally delete the
///     current version. The helper re-runs `apply` after deletion to establish
///     the caller-defined postcondition across delete + recreate ABA cycles.
///
/// `S: PartialEq` powers the equality fast path (skip the write when nothing
/// changed); `S: Clone` lets each retry hand `apply` an owned snapshot while the
/// helper retains a copy for that equality check — both mirror the turns
/// reference.
pub async fn cas_update<F, S, T, E, D, N, A, Fut>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    decode: D,
    encode: N,
    mut apply: A,
) -> Result<T, CasUpdateError<E>>
where
    F: RootFilesystem + ?Sized,
    S: PartialEq + Clone,
    D: Fn(&[u8]) -> Result<S, E>,
    N: Fn(&S) -> Result<Entry, E>,
    A: FnMut(Option<S>) -> Fut,
    Fut: Future<Output = Result<CasApply<S, T>, E>>,
{
    // Pre-flight capability gate. Only enforce when the backend advertises a
    // *known* shape; the composite router advertises the empty default and
    // defers to the op-time check below. Either way we never fall back to
    // `CasExpectation::Any`.
    let capabilities = filesystem.capabilities();
    if capabilities_known(&capabilities) && !capabilities_support_cas(&capabilities) {
        return Err(CasUpdateError::CasUnsupported);
    }

    let loop_future = cas_update_loop(
        filesystem,
        scope,
        path,
        capabilities,
        &decode,
        &encode,
        &mut apply,
    );
    match tokio::time::timeout(FILESYSTEM_APPLY_TIMEOUT, loop_future).await {
        Ok(result) => result,
        Err(_) => Err(CasUpdateError::Timeout),
    }
}

async fn cas_update_loop<F, S, T, E, D, N, A, Fut>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    capabilities: BackendCapabilities,
    decode: &D,
    encode: &N,
    apply: &mut A,
) -> Result<T, CasUpdateError<E>>
where
    F: RootFilesystem + ?Sized,
    S: PartialEq + Clone,
    D: Fn(&[u8]) -> Result<S, E>,
    N: Fn(&S) -> Result<Entry, E>,
    A: FnMut(Option<S>) -> Fut,
    Fut: Future<Output = Result<CasApply<S, T>, E>>,
{
    let mut mutation_attempts = 0;
    // A successful conditional delete must be verified by a fresh read/apply,
    // but that verification-only invocation must not replace the outcome from
    // the mutation that actually succeeded.
    let mut successful_delete_outcome = None;
    loop {
        // 1. Read the current versioned snapshot. Pure reads are lock-free; a
        //    reader racing a write observes either the previous or next
        //    committed version, never a torn one.
        let (current, version) = match filesystem.get(scope, path).await {
            Ok(Some(versioned)) => {
                let decoded = decode(&versioned.entry.body).map_err(CasUpdateError::Apply)?;
                (Some(decoded), Some(versioned.version))
            }
            Ok(None) => (None, None),
            Err(error) => return Err(CasUpdateError::Backend(error)),
        };

        // 2. Run the caller's transform against the freshly read snapshot.
        //    `apply` is re-runnable on every retry, so it receives an owned
        //    clone while `current` is retained for the no-op equality check.
        let CasApply {
            snapshot,
            outcome,
            operation,
        } = apply(current.clone())
            .await
            .map_err(CasUpdateError::Apply)?;

        // 3a. Explicit no-op: caller returned CasApply::no_op — skip the write
        //     unconditionally. This handles the absent-record + default-snapshot
        //     case where there is no `existing` to compare against.
        if operation == CasApplyOperation::NoOp {
            return Ok(successful_delete_outcome.unwrap_or(outcome));
        }

        // 3b. Equality fast-path: snapshot is unchanged from what `apply`
        //     received, so skip the write.
        if operation == CasApplyOperation::Write
            && matches!(&current, Some(existing) if *existing == snapshot)
        {
            return Ok(successful_delete_outcome.unwrap_or(outcome));
        }

        if operation == CasApplyOperation::Delete && current.is_none() {
            return Ok(successful_delete_outcome.unwrap_or(outcome));
        }
        if operation == CasApplyOperation::Delete
            && capabilities_known(&capabilities)
            && !capabilities.has(Capability::Delete)
        {
            return Err(CasUpdateError::CasUnsupported);
        }
        if mutation_attempts >= FILESYSTEM_CAS_RETRIES {
            return Err(CasUpdateError::RetriesExhausted);
        }
        let attempt = mutation_attempts;
        mutation_attempts += 1;

        match operation {
            CasApplyOperation::Write => {
                // Verification found that a different mutation is now needed;
                // its outcome supersedes any earlier delete outcome.
                drop(successful_delete_outcome.take());
                // 4a. Encode + CAS put. Absent → create-if-absent; existing →
                //     version precondition.
                let entry = encode(&snapshot).map_err(CasUpdateError::Apply)?;
                let cas = match version {
                    Some(version) => CasExpectation::Version(version),
                    None => CasExpectation::Absent,
                };
                match filesystem.put(scope, path, entry, cas).await {
                    Ok(_) => return Ok(outcome),
                    // 5a. Lost the CAS race — re-read and retry with backoff.
                    Err(FilesystemError::VersionMismatch { .. }) => {
                        if mutation_attempts < FILESYSTEM_CAS_RETRIES {
                            cas_retry_backoff(attempt).await;
                        }
                    }
                    // 5b. Backend cannot CAS-write — fail closed (no blind
                    //     overwrite).
                    Err(FilesystemError::Unsupported {
                        operation: FilesystemOperation::WriteFile,
                        ..
                    }) => return Err(CasUpdateError::CasUnsupported),
                    Err(error) => return Err(CasUpdateError::Backend(error)),
                }
            }
            CasApplyOperation::Delete => {
                let Some(expected_version) = version else {
                    return Ok(successful_delete_outcome.unwrap_or(outcome));
                };
                match filesystem
                    .delete_if_version(scope, path, expected_version)
                    .await
                {
                    // Never return directly after delete. Re-read and re-apply
                    // the caller's postcondition so delete + recreate cannot
                    // win an ABA race with the captured version.
                    Ok(()) => successful_delete_outcome = Some(outcome),
                    Err(FilesystemError::NotFound { .. })
                    | Err(FilesystemError::VersionMismatch { .. }) => {
                        if mutation_attempts < FILESYSTEM_CAS_RETRIES {
                            cas_retry_backoff(attempt).await;
                        }
                    }
                    Err(FilesystemError::Unsupported {
                        operation: FilesystemOperation::Delete,
                        ..
                    }) => return Err(CasUpdateError::CasUnsupported),
                    Err(error) => return Err(CasUpdateError::Backend(error)),
                }
            }
            CasApplyOperation::NoOp => {
                return Ok(successful_delete_outcome.unwrap_or(outcome));
            }
        }
    }
}

/// `true` when the backend advertised a non-default capability shape and we can
/// therefore trust the pre-flight gate. The composite router and any
/// not-yet-overridden `capabilities()` return the empty default, which we treat
/// as "unknown, defer to op-time".
fn capabilities_known(capabilities: &BackendCapabilities) -> bool {
    *capabilities != BackendCapabilities::default()
}

/// `true` when the backend's transaction tier is at least
/// [`TxnCapability::Cas`].
fn capabilities_support_cas(capabilities: &BackendCapabilities) -> bool {
    matches!(
        capabilities.txn(),
        TxnCapability::Cas | TxnCapability::MultiKey
    )
}

/// Jittered exponential backoff between CAS retries.
///
/// 2ms base, doubling per attempt, capped at 50ms, plus up to one base-delay
/// of jitter to de-correlate competing writers. Jitter is derived from a
/// `RandomState`-seeded hash of the attempt index so it is uncorrelated across
/// callers and works correctly on coarse-clock platforms (VMs, containers,
/// Windows) where `SystemTime::now()` advances in 1ms or 15ms ticks and a
/// nanosecond-modulo approach would collapse to zero on every retry.
async fn cas_retry_backoff(attempt: usize) {
    let shift = attempt.min(8) as u32;
    let multiplier = 1_u32.checked_shl(shift).unwrap_or(u32::MAX);
    let base_delay = FILESYSTEM_CAS_BACKOFF_BASE
        .saturating_mul(multiplier)
        .min(FILESYSTEM_CAS_BACKOFF_MAX);
    let jitter = {
        use std::collections::hash_map::RandomState;
        use std::hash::BuildHasher;
        let hash = RandomState::new().hash_one(attempt);
        let jitter_ceiling = base_delay.as_millis().max(1) as u64;
        Duration::from_millis(hash % jitter_ceiling)
    };
    tokio::time::sleep(base_delay.saturating_add(jitter)).await;
}

#[cfg(test)]
mod tests;
