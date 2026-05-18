//! Per-caller concurrency cap on long-lived SSE streams.
//!
//! The route descriptor's [`RateLimitPolicy`] bounds the rate at which
//! a caller can *open new* SSE connections, but it does not bound the
//! number of concurrent streams a caller holds open. Without a cap on
//! concurrent streams, an authenticated caller could open one stream
//! per rate-limit window and never close them, multiplying backend
//! projection drains at `connections × poll-interval` indefinitely.
//!
//! This module gates `stream_events` with a per-caller concurrent cap.
//! Slots are reserved synchronously when the handler runs and released
//! automatically when the underlying SSE stream is dropped (client
//! disconnect, max-lifetime reached, or facade error).
//!
//! [`RateLimitPolicy`]: ironclaw_host_api::ingress::RateLimitPolicy

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ironclaw_host_api::{TenantId, UserId};

/// Default concurrent SSE streams per (tenant, user). Sized to cover a
/// normal browser tab plus brief reconnect overlap; sustained abuse hits
/// the cap and gets 429.
pub(crate) const DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER: usize = 3;

/// Maximum lifetime of a single SSE stream before the handler closes it
/// cleanly so the browser can reconnect with `Last-Event-ID`. Bounds
/// drift between the projection cursor and any stale handler state, and
/// gives the per-caller cap a periodic floor to recover from leaked
/// guards in adverse conditions.
pub(crate) const SSE_MAX_LIFETIME: std::time::Duration = std::time::Duration::from_secs(5 * 60);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CallerKey {
    tenant_id: TenantId,
    user_id: UserId,
}

#[derive(Debug)]
pub(crate) struct SseCapacity {
    state: Mutex<HashMap<CallerKey, usize>>,
    max_per_caller: usize,
}

impl SseCapacity {
    pub(crate) fn new(max_per_caller: usize) -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
            max_per_caller,
        }
    }

    /// Reserve one slot for the given caller. Returns `None` if the
    /// caller is at or above [`Self::max_per_caller`]. Drop the returned
    /// guard to release the slot.
    pub(crate) fn try_acquire(
        self: &Arc<Self>,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Option<SseSlot> {
        let key = CallerKey {
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
        };
        let mut state = self.state.lock().expect("SseCapacity state lock poisoned"); // safety: only this module locks; no nested locks; release is infallible
        let entry = state.entry(key.clone()).or_insert(0);
        if *entry >= self.max_per_caller {
            return None;
        }
        *entry += 1;
        Some(SseSlot {
            capacity: Arc::clone(self),
            key,
        })
    }

    fn release(&self, key: &CallerKey) {
        let mut state = self.state.lock().expect("SseCapacity state lock poisoned"); // safety: only this module locks; no nested locks
        if let Some(count) = state.get_mut(key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.remove(key);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn open_count(&self, tenant_id: &TenantId, user_id: &UserId) -> usize {
        let key = CallerKey {
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
        };
        let state = self.state.lock().expect("SseCapacity state lock poisoned"); // safety: test-only inspection
        state.get(&key).copied().unwrap_or(0)
    }
}

/// RAII reservation for one SSE stream slot.
///
/// The slot is held by the SSE handler's async generator for the lifetime
/// of the stream and dropped automatically when the generator is dropped
/// — client disconnect, max-lifetime expiry, or facade error.
#[derive(Debug)]
pub(crate) struct SseSlot {
    capacity: Arc<SseCapacity>,
    key: CallerKey,
}

impl Drop for SseSlot {
    fn drop(&mut self) {
        self.capacity.release(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant() -> TenantId {
        TenantId::new("tenant-1").expect("tenant")
    }

    fn user(name: &str) -> UserId {
        UserId::new(name).expect("user")
    }

    #[test]
    fn acquires_up_to_cap_then_refuses() {
        let cap = Arc::new(SseCapacity::new(2));
        let alice = user("alice");
        let s1 = cap.try_acquire(&tenant(), &alice).expect("first slot");
        let s2 = cap.try_acquire(&tenant(), &alice).expect("second slot");
        assert!(
            cap.try_acquire(&tenant(), &alice).is_none(),
            "third slot must be refused"
        );
        assert_eq!(cap.open_count(&tenant(), &alice), 2);
        drop(s1);
        // After release, a new slot is available again.
        let s3 = cap
            .try_acquire(&tenant(), &alice)
            .expect("slot after release");
        drop(s2);
        drop(s3);
        assert_eq!(cap.open_count(&tenant(), &alice), 0);
    }

    #[test]
    fn separate_callers_have_independent_caps() {
        let cap = Arc::new(SseCapacity::new(1));
        let alice = user("alice");
        let bob = user("bob");
        let _alice_slot = cap.try_acquire(&tenant(), &alice).expect("alice");
        let _bob_slot = cap.try_acquire(&tenant(), &bob).expect("bob");
        assert!(cap.try_acquire(&tenant(), &alice).is_none());
        assert!(cap.try_acquire(&tenant(), &bob).is_none());
    }
}
