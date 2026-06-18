//! Cross-process per-account serialization for OAuth credential refresh.
//!
//! # Two-layer concurrency ownership
//!
//! This wrapper owns the **cross-process** serialization contract:
//! it acquires a Postgres session-scoped advisory lock (a BLOCKING
//! `pg_advisory_lock`) keyed on the `CredentialAccountId` UUID before
//! delegating to the inner refresh port. If another process already holds
//! the lock for the same account, this wrapper WAITS until that process
//! finishes, then refreshes — by which point the winner has persisted its
//! (possibly rotated) refresh token, so the serialized second refresh reads
//! the fresh token from the store and cannot hit `invalid_grant`. Two
//! processes therefore never reach Google's token endpoint for the same
//! account simultaneously.
//!
//! The pre-existing in-process `refresh_locks` on
//! `ProviderBackedCredentialAccountService` (ironclaw_auth::credential)
//! is **retained** as a strictly *intra-process* stampede guard: it
//! prevents multiple tokio tasks inside one process from all racing to
//! the token endpoint.  That is a local optimization layered _under_
//! this wrapper, not a competing source of truth.  Do not remove it.
//!
//! # libsql / no-pool path
//!
//! When the pool is `None` (libsql, local-dev), this wrapper is a pure
//! identity pass-through: all refresh calls reach the inner port
//! unconditionally.  libsql is single-writer by deployment topology so
//! no cross-process lock is needed.
//!
//! # Infra-error fallback
//!
//! If the Postgres pool returns a connection error, or the `pg_advisory_lock`
//! query itself fails, this wrapper logs at `debug!` and proceeds to
//! the inner refresh without the lock.  Availability trumps strict
//! serialization: a transient infrastructure failure must not silently
//! block all refreshes.  The intra-process lock still guards against
//! local stampede.

// Key-derivation note:
// `advisory_lock_key_bytes` below uses the same first-8-bytes-as-two-i32 scheme
// as the canonical `advisory_lock_key_from_bytes` in
// `crates/ironclaw_hooks_postgres/src/backend.rs:701`.
// The two uses are in **disjoint namespaces** (credential-refresh vs.
// hooks-predicate-eviction), so they never alias.  If `ironclaw_hooks_postgres`
// is ever added as a dep of this crate, replace this local helper with a
// call to the exported version.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{AuthProductError, CredentialRefreshReport, CredentialRefreshRequest};
#[cfg(feature = "postgres")]
use tracing::debug;

use crate::product_auth_runtime_credentials::RuntimeCredentialAccountRefreshPort;

// ---------------------------------------------------------------------------
// Advisory lock key derivation
// ---------------------------------------------------------------------------

/// Derive the `(i32, i32)` advisory-lock key from an arbitrary byte slice.
#[cfg_attr(not(any(feature = "postgres", test)), allow(dead_code))]
///
/// Uses the same scheme as `advisory_lock_key_from_bytes` in
/// `crates/ironclaw_hooks_postgres/src/backend.rs:701` — first 4 bytes → `a`,
/// next 4 bytes → `b`, LE interpretation, zero-padded if the slice is short.
///
/// A hash collision across distinct account IDs merely causes two unrelated
/// accounts to serialize with each other — a rare throughput cost, never a
/// correctness bug.
///
/// For `CredentialAccountId` (UUID = 16 bytes) the first 8 bytes are always
/// present, so zero-padding never occurs in practice.
pub(crate) fn advisory_lock_key_bytes(key: &[u8]) -> (i32, i32) {
    let mut buf = [0u8; 8];
    let n = key.len().min(8);
    buf[..n].copy_from_slice(&key[..n]);
    let a = i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let b = i32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    (a, b)
}

// ---------------------------------------------------------------------------
// Advisory-locked wrapper
// ---------------------------------------------------------------------------

/// Wraps a [`RuntimeCredentialAccountRefreshPort`] with a Postgres
/// session-scoped advisory lock so that only one process at a time can
/// refresh a given credential account.
///
/// Constructed by the composition root (`auth.rs`) via
/// [`AdvisoryLockedCredentialRefresh::new`]; the pool is always `None` on the
/// libsql path so the wrapper degrades to a pass-through.
pub(crate) struct AdvisoryLockedCredentialRefresh {
    inner: Arc<dyn RuntimeCredentialAccountRefreshPort>,
    /// Postgres pool for advisory-lock acquisition.  `None` on the libsql /
    /// local-dev path → pure pass-through.
    #[cfg(feature = "postgres")]
    pool: Option<deadpool_postgres::Pool>,
}

impl AdvisoryLockedCredentialRefresh {
    /// Create a wrapper with a Postgres pool.  On the libsql path, call
    /// [`Self::passthrough`] instead.
    #[cfg(feature = "postgres")]
    pub(crate) fn new(
        inner: Arc<dyn RuntimeCredentialAccountRefreshPort>,
        pool: deadpool_postgres::Pool,
    ) -> Self {
        Self {
            inner,
            pool: Some(pool),
        }
    }

    /// Create a pass-through wrapper with no pool.  All refresh calls reach
    /// `inner` unconditionally (libsql / local-dev path).
    /// Used in tests and the non-postgres path.
    #[allow(dead_code)]
    pub(crate) fn passthrough(inner: Arc<dyn RuntimeCredentialAccountRefreshPort>) -> Self {
        Self {
            inner,
            #[cfg(feature = "postgres")]
            pool: None,
        }
    }
}

#[async_trait]
impl RuntimeCredentialAccountRefreshPort for AdvisoryLockedCredentialRefresh {
    async fn refresh_credential_account(
        &self,
        request: CredentialRefreshRequest,
    ) -> Result<CredentialRefreshReport, AuthProductError> {
        #[cfg(feature = "postgres")]
        {
            if let Some(pool) = &self.pool {
                return refresh_with_advisory_lock(&self.inner, pool, request).await;
            }
        }
        // No pool (libsql / local-dev): identity pass-through.
        self.inner.refresh_credential_account(request).await
    }
}

// ---------------------------------------------------------------------------
// Postgres advisory-lock acquisition logic (postgres-feature only)
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
async fn refresh_with_advisory_lock(
    inner: &Arc<dyn RuntimeCredentialAccountRefreshPort>,
    pool: &deadpool_postgres::Pool,
    request: CredentialRefreshRequest,
) -> Result<CredentialRefreshReport, AuthProductError> {
    let account_id = request.account_id;
    let (key_a, key_b) = advisory_lock_key_bytes(account_id.as_uuid().as_bytes());

    // Obtain a connection from the pool.  On failure, fall through to inner
    // without the lock (availability over strict serialization — see module doc).
    let conn = match pool.get().await {
        Ok(c) => c,
        Err(err) => {
            debug!(
                account_id = %account_id,
                error = %err,
                "advisory-lock pool error; proceeding without cross-process lock"
            );
            return inner.refresh_credential_account(request).await;
        }
    };

    // Acquire the session-scoped advisory lock with a BLOCKING wait.
    // `pg_advisory_lock` blocks until the lock is free, so a second process
    // serializes BEHIND the first rather than racing it to the token endpoint.
    // This is the property the issue requires: concurrent processes cannot
    // refresh the same account simultaneously. The wait is bounded by a single
    // refresh's duration (the holder releases as soon as its inner refresh
    // returns), and Postgres releases session-scoped advisory locks automatically
    // if a holding session's connection drops (e.g. the holder process crashes),
    // so there is no indefinite-wait / stuck-holder hazard.
    //
    // Correctness of the serialized second refresh: by the time we acquire the
    // lock, any concurrent winner has finished and persisted its (possibly
    // rotated) refresh token. Our inner refresh re-reads the refresh token from
    // the secret store, so it uses the fresh token and cannot hit invalid_grant.
    // The second refresh is redundant but safe (we do not skip it, because the
    // inline caller needs a valid access token returned now; the upstream
    // margin check in product_auth_runtime_credentials already suppresses most
    // redundant refreshes before reaching this port).
    if let Err(err) = conn
        .query_one("SELECT pg_advisory_lock($1, $2)", &[&key_a, &key_b])
        .await
    {
        debug!(
            account_id = %account_id,
            error = %err,
            "pg_advisory_lock wait failed; proceeding without cross-process lock"
        );
        // Cannot acquire the lock — fail safe by proceeding (availability over
        // strict serialization; the in-process refresh_locks guard still
        // prevents intra-process stampede).
        return inner.refresh_credential_account(request).await;
    }

    // We hold the advisory lock.  Delegate to inner, then explicitly release
    // the lock regardless of outcome.
    let result = inner.refresh_credential_account(request).await;

    // Release the lock explicitly so the connection can be returned to the pool
    // without holding it for its full session lifetime.
    if let Err(err) = conn
        .query_one("SELECT pg_advisory_unlock($1, $2)", &[&key_a, &key_b])
        .await
    {
        // Unlock failure is non-fatal: the lock will be released when the
        // connection is dropped / returned to pool.
        debug!(
            account_id = %account_id,
            error = %err,
            "pg_advisory_unlock query failed; lock will release on connection drop"
        );
    }

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_auth::{
        AuthProductError, AuthProviderId, AuthProductScope, AuthSurface, CredentialAccountId,
        CredentialRefreshReport, CredentialRefreshRequest,
    };
    use ironclaw_host_api::{InvocationId, ResourceScope, TenantId, UserId};

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    fn make_request() -> CredentialRefreshRequest {
        let scope = AuthProductScope::new(
            ResourceScope {
                tenant_id: TenantId::new("tenant-test").expect("tenant"),
                user_id: UserId::new("user-test").expect("user"),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            AuthSurface::Api,
        );
        let provider = AuthProviderId::new("google").expect("provider");
        let account_id = CredentialAccountId::new();
        CredentialRefreshRequest::new(scope, provider, account_id)
    }

    // A minimal stub that counts how many times it was called.
    struct CountingRefreshPort {
        calls: Arc<AtomicUsize>,
    }

    impl CountingRefreshPort {
        fn new() -> (Self, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            (Self { calls: calls.clone() }, calls)
        }
    }

    #[async_trait]
    impl RuntimeCredentialAccountRefreshPort for CountingRefreshPort {
        async fn refresh_credential_account(
            &self,
            _request: CredentialRefreshRequest,
        ) -> Result<CredentialRefreshReport, AuthProductError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(AuthProductError::BackendUnavailable)
        }
    }

    // ---------------------------------------------------------------------------
    // Key derivation tests
    // ---------------------------------------------------------------------------

    /// Same byte input always produces the same (a, b) pair.
    #[test]
    fn key_derivation_deterministic() {
        let id = CredentialAccountId::new();
        let bytes = id.as_uuid().as_bytes().to_vec();
        let k1 = advisory_lock_key_bytes(&bytes);
        let k2 = advisory_lock_key_bytes(&bytes);
        assert_eq!(k1, k2);
    }

    /// Two different account_ids should (with overwhelming probability) produce
    /// different keys — they are different 16-byte UUIDs so the first 8 bytes
    /// virtually never collide.
    #[test]
    fn key_derivation_different_ids_differ() {
        let id1 = CredentialAccountId::new();
        let id2 = CredentialAccountId::new();
        let k1 = advisory_lock_key_bytes(id1.as_uuid().as_bytes());
        let k2 = advisory_lock_key_bytes(id2.as_uuid().as_bytes());
        // UUIDs are random v4 — extremely unlikely to collide in the first 8 bytes.
        assert_ne!(k1, k2, "two distinct UUIDs produced the same advisory key");
    }

    /// Zero-padding: a 4-byte key still produces a stable, total result.
    #[test]
    fn key_derivation_short_slice() {
        let k = advisory_lock_key_bytes(b"abcd");
        assert_eq!(k, (i32::from_le_bytes(*b"abcd"), 0i32));
    }

    /// Empty slice: both halves are zero.
    #[test]
    fn key_derivation_empty_slice() {
        let k = advisory_lock_key_bytes(b"");
        assert_eq!(k, (0i32, 0i32));
    }

    // ---------------------------------------------------------------------------
    // Pass-through path (no pool)
    // ---------------------------------------------------------------------------

    /// With no pool the inner service is always called.
    #[tokio::test]
    async fn passthrough_always_calls_inner() {
        let (port, calls) = CountingRefreshPort::new();
        let wrapper = AdvisoryLockedCredentialRefresh::passthrough(Arc::new(port));

        // Call twice.
        let _ = wrapper.refresh_credential_account(make_request()).await;
        let _ = wrapper.refresh_credential_account(make_request()).await;

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
