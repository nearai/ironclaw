//! Process-local nonce store for the NEAR wallet challenge/verify flow.
//!
//! Each `GET /auth/near/challenge` mints a random 32-byte nonce
//! (returned hex-encoded) and persists it; the matching
//! `POST /auth/near/verify` `consume`s it. Single-use on `consume`,
//! so a replayed verify with an already-spent nonce fails closed.
//! Bounded (capacity cap + TTL) so a flood of unauthenticated
//! challenge requests cannot grow the map unbounded — the cap is
//! enforced before insertion with opportunistic GC, mirroring the
//! `PendingFlowStore` in `auth/pending.rs`.
//!
//! The cache is intentionally process-local. A future multi-replica
//! deployment must replace this with a shared store, exactly like the
//! OAuth pending-flow / session-ticket stores it sits beside.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rand::RngCore;
use rand::rngs::OsRng;

/// Challenge nonces older than this are rejected on `consume` and
/// swept on insert. Matches the v1 gateway's 5-minute window.
const NONCE_TTL: Duration = Duration::from_secs(300);
/// Hard cap on outstanding nonces to bound memory under flood.
const MAX_NONCES: usize = 1024;

/// Thread-safe, bounded, single-use nonce store.
#[derive(Default)]
pub(crate) struct NearNonceStore {
    inner: Mutex<HashMap<String, Instant>>,
}

impl NearNonceStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Generate and store a random 32-byte nonce, returned hex-encoded
    /// (64 chars). The hex form round-trips cleanly through the
    /// challenge JSON and back on verify, where it is decoded to the
    /// raw 32 bytes the NEP-413 payload binds.
    pub(crate) fn generate(&self) -> String {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let nonce = hex::encode(bytes);

        let mut guard = self.inner.lock();
        // Opportunistic GC on insert: at capacity, sweep expired
        // entries first, then drop the oldest if still full. Keeps the
        // map bounded under flood without a background task.
        if guard.len() >= MAX_NONCES {
            guard.retain(|_, created| created.elapsed() < NONCE_TTL);
        }
        if guard.len() >= MAX_NONCES
            && let Some(oldest) = guard
                .iter()
                .min_by_key(|(_, created)| **created)
                .map(|(k, _)| k.clone())
        {
            guard.remove(&oldest);
        }
        guard.insert(nonce.clone(), Instant::now());
        nonce
    }

    /// Atomically remove `nonce` and report whether it was valid
    /// (present and not expired). Single-use: the entry is removed
    /// regardless, so a replayed verify cannot re-spend it.
    pub(crate) fn consume(&self, nonce: &str) -> bool {
        let mut guard = self.inner.lock();
        match guard.remove(nonce) {
            Some(created) => created.elapsed() < NONCE_TTL,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_then_consume_is_single_use() {
        let store = NearNonceStore::new();
        let nonce = store.generate();
        assert_eq!(nonce.len(), 64, "32 bytes hex-encoded");
        assert!(store.consume(&nonce), "first consume succeeds");
        assert!(!store.consume(&nonce), "replayed consume fails closed");
    }

    #[test]
    fn unknown_nonce_is_rejected() {
        let store = NearNonceStore::new();
        assert!(!store.consume("never-issued"));
    }

    #[test]
    fn expired_nonce_is_rejected_and_removed() {
        let store = NearNonceStore::new();
        {
            let mut guard = store.inner.lock();
            guard.insert(
                "stale".to_string(),
                Instant::now() - NONCE_TTL - Duration::from_secs(1),
            );
        }
        assert!(!store.consume("stale"), "expired nonce must fail closed");
        assert!(
            !store.inner.lock().contains_key("stale"),
            "expired nonce must be removed on consume",
        );
    }

    #[test]
    fn store_evicts_oldest_when_capacity_exceeded() {
        let store = NearNonceStore::new();
        let mut nonces = Vec::with_capacity(MAX_NONCES + 1);
        for _ in 0..=MAX_NONCES {
            nonces.push(store.generate());
        }
        let guard = store.inner.lock();
        assert!(guard.len() <= MAX_NONCES, "store must stay bounded");
        assert!(
            !guard.contains_key(&nonces[0]),
            "oldest nonce must be evicted",
        );
        assert!(
            guard.contains_key(nonces.last().expect("nonempty")),
            "newest nonce must survive",
        );
    }
}
