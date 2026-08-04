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
//! disconnect, max-lifetime reached, or service error).
//!
//! [`RateLimitPolicy`]: ironclaw_host_api::ingress::RateLimitPolicy

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ironclaw_host_api::ids::{TenantId, UserId};
use tokio::sync::watch;

/// Default concurrent SSE streams per (tenant, user). Sized to cover a
/// normal browser tab plus brief reconnect overlap; sustained abuse hits
/// the cap and gets 429.
pub const DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER: usize = 3;

/// Number of consecutive capacity-rejected SSE open attempts (while a
/// caller sits at the concurrency cap) that stay marked refundable against
/// `webui_rate_limit::enforce_rate_limit`'s request-volume budget, before
/// this module stops refunding and lets further attempts drain that budget
/// like any other request.
///
/// Without this bound, a caller who is already saturated could send
/// unlimited capacity-rejected opens and every single one would be
/// refunded — the per-caller request-volume limiter (whose whole job is
/// bounding request *volume*) would provide zero throttling for the rest
/// of the saturation episode (PR #6592 review). The cap is generous enough
/// to absorb ordinary reconnect racing (a browser `EventSource` retrying
/// while an old stream hasn't yet closed) without penalizing it, while
/// still bounding a saturated caller's free-429 hammer: once a streak
/// crosses this limit, further rejections are ordinary (non-refunded)
/// charges against the route's configured request-volume budget, same as
/// any other request.
const REJECTION_REFUND_LIMIT: u32 = 5;

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
    state: Mutex<CapacityState>,
    max_per_caller: usize,
    next_generation: AtomicU64,
}

#[derive(Debug, Default)]
struct CapacityState {
    open_by_caller: HashMap<CallerKey, CallerState>,
    named_slots: HashMap<(CallerKey, String), NamedSlot>,
}

#[derive(Debug, Default)]
struct CallerState {
    /// Number of currently held slots.
    open: usize,
    /// Consecutive capacity-rejected attempts since this caller last
    /// successfully acquired a slot. Reset to 0 on every successful
    /// acquire; bounds how many rejections in a row are reported as
    /// refundable — see [`REJECTION_REFUND_LIMIT`].
    rejected_streak: u32,
}

#[derive(Debug)]
struct NamedSlot {
    generation: u64,
    client_generation: Option<u64>,
    cancel: watch::Sender<bool>,
}

#[derive(Debug)]
pub(crate) enum SseAcquireResult {
    Acquired(SseSlot),
    /// The caller is at or above the concurrency cap. `refundable` says
    /// whether this specific rejection should be exempted from
    /// `webui_rate_limit::enforce_rate_limit`'s request-volume charge —
    /// see [`REJECTION_REFUND_LIMIT`].
    AtCapacity {
        refundable: bool,
    },
    StaleGeneration,
}

impl SseCapacity {
    pub(crate) fn new(max_per_caller: usize) -> Self {
        Self {
            state: Mutex::new(CapacityState::default()),
            max_per_caller,
            next_generation: AtomicU64::new(1),
        }
    }

    /// Reserve one slot for the given caller, or report a capacity
    /// rejection (with whether it should be refunded — see
    /// [`REJECTION_REFUND_LIMIT`]) if the caller is at or above
    /// [`Self::max_per_caller`]. Drop the returned guard to release the
    /// slot.
    pub(crate) fn try_acquire(
        self: &Arc<Self>,
        tenant_id: &TenantId,
        user_id: &UserId,
        connection_id: Option<&str>,
    ) -> SseAcquireResult {
        self.try_acquire_ordered(tenant_id, user_id, connection_id, None)
    }

    /// Reserve a slot using the browser tab's monotonically increasing stream
    /// generation. A delayed request from an older route must not cancel the
    /// newer route merely because it reached the server later.
    pub(crate) fn try_acquire_ordered(
        self: &Arc<Self>,
        tenant_id: &TenantId,
        user_id: &UserId,
        connection_id: Option<&str>,
        client_generation: Option<u64>,
    ) -> SseAcquireResult {
        // A configured cap of 0 (SSE disabled) is not special-cased: with
        // `open` starting at 0, `entry.open >= self.max_per_caller` is
        // immediately true, so cap-zero callers fall straight into the
        // same saturated-rejection branch below and get the same
        // `rejected_streak` / `REJECTION_REFUND_LIMIT` bookkeeping as any
        // other saturated caller. An earlier version special-cased
        // `max_per_caller == 0` with an early return before this
        // bookkeeping ran, which meant every cap-zero rejection was
        // reported refundable forever — see [`REJECTION_REFUND_LIMIT`]'s
        // doc comment. Cap-zero callers can never successfully acquire
        // (the `open >= max_per_caller` check never lets them through), so
        // their per-caller entry is never released and its
        // `rejected_streak` accumulates for the life of the process; that
        // is the intended trade-off to keep this rejection path free of
        // free 429s, and no production profile configures a cap of 0
        // today.
        let key = CallerKey {
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
        };
        let mut state = lock_state(&self.state);
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        if let Some(connection_id) = connection_id {
            let named_key = (key.clone(), connection_id.to_string());
            if let Some(previous) = state.named_slots.get(&named_key) {
                match (client_generation, previous.client_generation) {
                    (Some(incoming), Some(current)) if incoming < current => {
                        return SseAcquireResult::StaleGeneration;
                    }
                    (None, Some(_)) => return SseAcquireResult::StaleGeneration,
                    _ => {}
                }
                let _ = previous.cancel.send(true);
                let (cancel, cancellation) = watch::channel(false);
                state.named_slots.insert(
                    named_key,
                    NamedSlot {
                        generation,
                        client_generation,
                        cancel,
                    },
                );
                // A named-slot replacement is a successful acquire (it
                // reuses the caller's existing reservation rather than
                // taking a new one), so it ends any rejection streak.
                if let Some(entry) = state.open_by_caller.get_mut(&key) {
                    entry.rejected_streak = 0;
                }
                return SseAcquireResult::Acquired(SseSlot {
                    capacity: Arc::clone(self),
                    key,
                    connection_id: Some(connection_id.to_string()),
                    generation,
                    cancellation: Some(cancellation),
                });
            }
        }
        let entry = state.open_by_caller.entry(key.clone()).or_default();
        if entry.open >= self.max_per_caller {
            entry.rejected_streak = entry.rejected_streak.saturating_add(1);
            let refundable = entry.rejected_streak <= REJECTION_REFUND_LIMIT;
            return SseAcquireResult::AtCapacity { refundable };
        }
        entry.open += 1;
        entry.rejected_streak = 0;
        let (connection_id, cancellation) = if let Some(connection_id) = connection_id {
            let (cancel, cancellation) = watch::channel(false);
            state.named_slots.insert(
                (key.clone(), connection_id.to_string()),
                NamedSlot {
                    generation,
                    client_generation,
                    cancel,
                },
            );
            (Some(connection_id.to_string()), Some(cancellation))
        } else {
            (None, None)
        };
        SseAcquireResult::Acquired(SseSlot {
            capacity: Arc::clone(self),
            key,
            connection_id,
            generation,
            cancellation,
        })
    }

    fn release(&self, key: &CallerKey, connection_id: Option<&str>, generation: u64) {
        let mut state = lock_state(&self.state);
        if let Some(connection_id) = connection_id {
            let named_key = (key.clone(), connection_id.to_string());
            let is_current = state
                .named_slots
                .get(&named_key)
                .is_some_and(|slot| slot.generation == generation);
            if !is_current {
                return;
            }
            state.named_slots.remove(&named_key);
        }
        if let Some(entry) = state.open_by_caller.get_mut(key) {
            entry.open = entry.open.saturating_sub(1);
            if entry.open == 0 {
                // No slots left — drop the whole entry (including any
                // stale `rejected_streak`) rather than let it linger
                // unbounded. The caller is no longer saturated, so the
                // next attempt succeeds and starts a fresh streak anyway.
                state.open_by_caller.remove(key);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn open_count(&self, tenant_id: &TenantId, user_id: &UserId) -> usize {
        let key = CallerKey {
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
        };
        let state = lock_state(&self.state);
        state
            .open_by_caller
            .get(&key)
            .map(|entry| entry.open)
            .unwrap_or(0)
    }
}

/// Acquire the slot-count map without ever panicking on a poisoned
/// mutex.
///
/// `SseSlot::drop` calls `SseCapacity::release`, so if any code path on
/// this lock had previously panicked while holding the guard, an
/// `expect`-on-poison would re-panic *inside* a Drop. During unwinding
/// from another panic that becomes a double-panic and the process
/// aborts — which is exactly the failure mode we never want for a
/// per-connection cleanup hook.
///
/// Recovering with `into_inner()` is safe here because the only data
/// behind the lock is a `HashMap<CallerKey, CallerState>` and every critical
/// section is a few lines of straight-line code that mutates a single
/// counter — there is no compound invariant for a mid-mutation panic to
/// break. The worst case is a single caller's count being off by one,
/// which `SSE_MAX_LIFETIME`-driven slot recycling self-heals within
/// minutes.
fn lock_state(mutex: &Mutex<CapacityState>) -> std::sync::MutexGuard<'_, CapacityState> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// RAII reservation for one SSE stream slot.
///
/// The slot is held by the SSE handler's async generator for the lifetime
/// of the stream and dropped automatically when the generator is dropped
/// — client disconnect, max-lifetime expiry, or service error.
#[derive(Debug)]
pub(crate) struct SseSlot {
    capacity: Arc<SseCapacity>,
    key: CallerKey,
    connection_id: Option<String>,
    generation: u64,
    cancellation: Option<watch::Receiver<bool>>,
}

impl SseSlot {
    pub(crate) async fn cancelled(&mut self) {
        let Some(cancellation) = self.cancellation.as_mut() else {
            std::future::pending::<()>().await;
            return;
        };
        if *cancellation.borrow() {
            return;
        }
        while cancellation.changed().await.is_ok() {
            if *cancellation.borrow() {
                return;
            }
        }
        std::future::pending::<()>().await;
    }

    #[cfg(test)]
    fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(|cancellation| *cancellation.borrow())
    }
}

impl Drop for SseSlot {
    fn drop(&mut self) {
        self.capacity
            .release(&self.key, self.connection_id.as_deref(), self.generation);
    }
}

#[cfg(test)]
impl SseAcquireResult {
    fn acquired(self) -> Option<SseSlot> {
        match self {
            SseAcquireResult::Acquired(slot) => Some(slot),
            SseAcquireResult::AtCapacity { .. } | SseAcquireResult::StaleGeneration => None,
        }
    }

    fn rejected_refundable(&self) -> Option<bool> {
        match self {
            SseAcquireResult::AtCapacity { refundable } => Some(*refundable),
            SseAcquireResult::Acquired(_) | SseAcquireResult::StaleGeneration => None,
        }
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
        let s1 = cap
            .try_acquire(&tenant(), &alice, None)
            .acquired()
            .expect("first slot");
        let s2 = cap
            .try_acquire(&tenant(), &alice, None)
            .acquired()
            .expect("second slot");
        assert!(
            cap.try_acquire(&tenant(), &alice, None)
                .acquired()
                .is_none(),
            "third slot must be refused"
        );
        assert_eq!(cap.open_count(&tenant(), &alice), 2);
        drop(s1);
        // After release, a new slot is available again.
        let s3 = cap
            .try_acquire(&tenant(), &alice, None)
            .acquired()
            .expect("slot after release");
        drop(s2);
        drop(s3);
        assert_eq!(cap.open_count(&tenant(), &alice), 0);
    }

    #[test]
    fn zero_capacity_rejects_and_stops_refunding_after_the_streak_limit() {
        // PR #6592 review round 2: `max_per_caller == 0` used to short-circuit
        // *before* the rejection bookkeeping, so every cap-zero rejection was
        // reported refundable forever — a caller could hammer a disabled SSE
        // route at zero cost to the request-volume budget. Cap-zero now falls
        // through the same `rejected_streak` accounting as any other saturated
        // caller.
        let cap = Arc::new(SseCapacity::new(0));
        let alice = user("alice");
        for attempt in 1..=REJECTION_REFUND_LIMIT {
            let outcome = cap.try_acquire(&tenant(), &alice, None);
            assert!(
                matches!(outcome, SseAcquireResult::AtCapacity { .. }),
                "cap 0 never acquires"
            );
            assert_eq!(
                outcome.rejected_refundable(),
                Some(true),
                "attempt {attempt} is within the refundable streak"
            );
        }
        assert_eq!(
            cap.try_acquire(&tenant(), &alice, None)
                .rejected_refundable(),
            Some(false),
            "past the streak limit, rejections drain the request-volume budget"
        );
        assert_eq!(
            cap.open_count(&tenant(), &alice),
            0,
            "a rejected open never holds a slot"
        );
    }

    #[test]
    fn saturated_caller_stops_being_refunded_after_the_streak_limit() {
        // The same bound applies to an ordinary (non-zero) cap: a caller
        // pinned at the concurrency cap gets a short refundable burst — enough
        // to absorb normal `EventSource` reconnect racing — and then pays for
        // further attempts out of the route's request-volume budget.
        let cap = Arc::new(SseCapacity::new(1));
        let alice = user("alice");
        let _held = cap
            .try_acquire(&tenant(), &alice, None)
            .acquired()
            .expect("first slot");
        for attempt in 1..=REJECTION_REFUND_LIMIT {
            assert_eq!(
                cap.try_acquire(&tenant(), &alice, None)
                    .rejected_refundable(),
                Some(true),
                "attempt {attempt} is within the refundable streak"
            );
        }
        assert_eq!(
            cap.try_acquire(&tenant(), &alice, None)
                .rejected_refundable(),
            Some(false),
            "past the streak limit the rejection is charged like any request"
        );
    }

    #[test]
    fn successful_acquire_resets_the_rejection_streak() {
        let cap = Arc::new(SseCapacity::new(1));
        let alice = user("alice");
        let held = cap
            .try_acquire(&tenant(), &alice, None)
            .acquired()
            .expect("first slot");
        for _ in 0..=REJECTION_REFUND_LIMIT {
            let _ = cap.try_acquire(&tenant(), &alice, None);
        }
        drop(held);

        // Releasing the slot drops the caller entry entirely, so the next
        // acquire starts from a clean streak and the burst budget is whole
        // again — a caller that recovers is not punished for the episode.
        let _reacquired = cap
            .try_acquire(&tenant(), &alice, None)
            .acquired()
            .expect("slot after release");
        assert_eq!(
            cap.try_acquire(&tenant(), &alice, None)
                .rejected_refundable(),
            Some(true),
            "the first rejection after a successful acquire is refundable again"
        );
    }

    // Regression for the SSE-slot Drop poison-abort review (Medium):
    // `SseSlot::drop` calls `release`, and if `release`'s lock acquire
    // ever `expect`-ed on a poisoned mutex, a panic-while-unwinding
    // would double-panic and abort the process. Poison the mutex
    // deliberately via a panicking thread, then exercise both `release`
    // (via `SseSlot::drop`) and `try_acquire` to make sure neither
    // re-panics.
    #[test]
    fn poisoned_lock_does_not_double_panic_on_release_or_acquire() {
        let cap = Arc::new(SseCapacity::new(2));
        let alice = user("alice");
        let slot = cap
            .try_acquire(&tenant(), &alice, None)
            .acquired()
            .expect("first slot");

        // Poison the mutex by panicking while holding the guard. We
        // catch the panic so the test process survives — the goal is
        // to leave the mutex in `PoisonError`, not to crash the test.
        {
            let cap = Arc::clone(&cap);
            let join = std::thread::spawn(move || {
                let _guard = cap.state.lock().expect("acquire to poison");
                panic!("intentional panic to poison SseCapacity mutex");
            });
            let result = join.join();
            assert!(
                result.is_err(),
                "poisoning thread should have panicked, not returned"
            );
        }
        assert!(
            cap.state.is_poisoned(),
            "test prerequisite: the mutex must actually be poisoned for the regression to be meaningful"
        );

        // Drop the live slot — without poison recovery, `release` would
        // `expect`-panic here while we are *not* unwinding, which would
        // fail the test. With recovery, the slot returns cleanly.
        drop(slot);

        // And a fresh acquire on the poisoned lock must also succeed
        // rather than panic; this is the call-site that runs on every
        // new SSE open.
        let recovered = cap
            .try_acquire(&tenant(), &alice, None)
            .acquired()
            .expect("try_acquire must recover from a poisoned lock");
        drop(recovered);
    }

    #[test]
    fn separate_callers_have_independent_caps() {
        let cap = Arc::new(SseCapacity::new(1));
        let alice = user("alice");
        let bob = user("bob");
        let _alice_slot = cap
            .try_acquire(&tenant(), &alice, None)
            .acquired()
            .expect("alice");
        let _bob_slot = cap
            .try_acquire(&tenant(), &bob, None)
            .acquired()
            .expect("bob");
        assert!(
            cap.try_acquire(&tenant(), &alice, None)
                .acquired()
                .is_none()
        );
        assert!(cap.try_acquire(&tenant(), &bob, None).acquired().is_none());
    }

    #[test]
    fn named_slot_replaces_its_prior_generation_without_consuming_capacity() {
        let cap = Arc::new(SseCapacity::new(1));
        let alice = user("alice");
        let first = cap
            .try_acquire(&tenant(), &alice, Some("browser-tab"))
            .acquired()
            .expect("first named slot");
        let replacement = cap
            .try_acquire(&tenant(), &alice, Some("browser-tab"))
            .acquired()
            .expect("same browser tab replaces its stale stream");

        assert!(first.is_cancelled(), "the prior stream must be cancelled");
        assert!(!replacement.is_cancelled());
        assert_eq!(cap.open_count(&tenant(), &alice), 1);
        assert!(
            cap.try_acquire(&tenant(), &alice, Some("different-tab"))
                .acquired()
                .is_none(),
            "a different browser tab still respects the per-caller cap"
        );

        drop(first);
        assert_eq!(
            cap.open_count(&tenant(), &alice),
            1,
            "dropping the superseded generation must not release the replacement"
        );
        drop(replacement);
        assert_eq!(cap.open_count(&tenant(), &alice), 0);
    }

    #[test]
    fn ordered_named_slot_rejects_a_late_older_client_generation() {
        let cap = Arc::new(SseCapacity::new(1));
        let alice = user("alice");
        let first = match cap.try_acquire_ordered(&tenant(), &alice, Some("browser-tab"), Some(1)) {
            SseAcquireResult::Acquired(slot) => slot,
            result => panic!("first generation must be admitted: {result:?}"),
        };
        let current = match cap.try_acquire_ordered(&tenant(), &alice, Some("browser-tab"), Some(2))
        {
            SseAcquireResult::Acquired(slot) => slot,
            result => panic!("newer generation must be admitted: {result:?}"),
        };

        assert!(first.is_cancelled());
        assert!(!current.is_cancelled());
        assert!(matches!(
            cap.try_acquire_ordered(&tenant(), &alice, Some("browser-tab"), Some(1)),
            SseAcquireResult::StaleGeneration
        ));
        assert!(
            !current.is_cancelled(),
            "a late older request must not cancel the current route stream"
        );
        assert_eq!(cap.open_count(&tenant(), &alice), 1);
    }
}
