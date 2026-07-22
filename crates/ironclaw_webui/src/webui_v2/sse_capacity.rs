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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ironclaw_host_api::{TenantId, UserId};
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
pub(crate) struct SseCapacity {
    state: Mutex<CapacityState>,
    max_per_caller: usize,
    next_generation: AtomicU64,
}

#[derive(Debug, Default)]
struct CapacityState {
    callers: HashMap<CallerKey, CallerState>,
    named_slots: HashMap<(CallerKey, String), NamedSlot>,
}

#[derive(Debug)]
struct NamedSlot {
    generation: u64,
    client_generation: Option<u64>,
    cancel: watch::Sender<bool>,
}

/// Outcome of [`SseCapacity::try_acquire`].
#[derive(Debug)]
pub(crate) enum SseCapacityOutcome {
    /// A concurrency slot was reserved. Hold the guard for the stream's
    /// lifetime; dropping it releases the slot.
    Acquired(SseSlot),
    /// The caller is at or above the concurrency cap. `refundable` says
    /// whether this specific rejection should be exempted from
    /// `enforce_rate_limit`'s request-volume charge — see
    /// [`REJECTION_REFUND_LIMIT`].
    Rejected { refundable: bool },
    /// A delayed request from an older route generation arrived after its
    /// replacement. It must not cancel or displace the current stream.
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
    ) -> SseCapacityOutcome {
        match self.try_acquire_ordered(tenant_id, user_id, connection_id, None) {
            SseCapacityOutcome::StaleGeneration => {
                SseCapacityOutcome::Rejected { refundable: false }
            }
            outcome => outcome,
        }
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
    ) -> SseCapacityOutcome {
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
                        return SseCapacityOutcome::StaleGeneration;
                    }
                    (None, Some(_)) => return SseCapacityOutcome::StaleGeneration,
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
                state
                    .callers
                    .entry(key.clone())
                    .or_default()
                    .rejected_streak = 0;
                return SseCapacityOutcome::Acquired(SseSlot {
                    capacity: Arc::clone(self),
                    key,
                    connection_id: Some(connection_id.to_string()),
                    generation,
                    cancellation: Some(cancellation),
                });
            }
        }
        // A configured cap of 0 follows the same rejection-streak accounting
        // as ordinary saturation so it cannot receive unlimited refunds.
        let entry = state.callers.entry(key.clone()).or_default();
        if entry.open >= self.max_per_caller {
            entry.rejected_streak = entry.rejected_streak.saturating_add(1);
            let refundable = entry.rejected_streak <= REJECTION_REFUND_LIMIT;
            return SseCapacityOutcome::Rejected { refundable };
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
        SseCapacityOutcome::Acquired(SseSlot {
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
        if let Some(entry) = state.callers.get_mut(key) {
            entry.open = entry.open.saturating_sub(1);
            if entry.open == 0 {
                // No slots left — drop the whole entry (including any stale
                // rejection streak). The next saturation episode starts clean.
                state.callers.remove(key);
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
        state.callers.get(&key).map(|entry| entry.open).unwrap_or(0)
    }
}

/// Acquire the capacity registry without ever panicking on a poisoned mutex.
///
/// `SseSlot::drop` calls `SseCapacity::release`, so if any code path on
/// this lock had previously panicked while holding the guard, an
/// `expect`-on-poison would re-panic *inside* a Drop. During unwinding
/// from another panic that becomes a double-panic and the process
/// aborts — which is exactly the failure mode we never want for a
/// per-connection cleanup hook.
///
/// Recovering with `into_inner()` is preferable to aborting the process. The
/// critical sections update only the caller counter and its corresponding
/// named-slot entry, without invoking user code. In the unlikely event that a
/// panic leaves those values out of sync, `SSE_MAX_LIFETIME`-driven slot
/// recycling bounds the impact.
fn lock_state(mutex: &Mutex<CapacityState>) -> std::sync::MutexGuard<'_, CapacityState> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
impl SseCapacityOutcome {
    fn acquired(self) -> Option<SseSlot> {
        match self {
            SseCapacityOutcome::Acquired(slot) => Some(slot),
            SseCapacityOutcome::Rejected { .. } | SseCapacityOutcome::StaleGeneration => None,
        }
    }

    fn rejected_refundable(&self) -> Option<bool> {
        match self {
            SseCapacityOutcome::Acquired(_) | SseCapacityOutcome::StaleGeneration => None,
            SseCapacityOutcome::Rejected { refundable } => Some(*refundable),
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
    fn zero_capacity_rejects_without_incrementing_open_count() {
        // With max_per_caller=0 the caller can never successfully
        // acquire, so `open` must stay 0 across any number of rejected
        // opens — only `rejected_streak` bookkeeping (covered by
        // `zero_cap_rejections_stop_being_refundable_past_the_burst_limit`)
        // advances. Note this *does* leave a per-caller entry in the
        // HashMap (needed so the streak persists across calls) — that is
        // an intentional trade-off, not a leak this test guards against;
        // see the comment on the `entry.open >= self.max_per_caller`
        // check in `try_acquire`.
        let cap = Arc::new(SseCapacity::new(0));
        let alice = user("alice");
        assert!(
            cap.try_acquire(&tenant(), &alice, None)
                .acquired()
                .is_none()
        );
        assert_eq!(
            cap.open_count(&tenant(), &alice),
            0,
            "rejected open must never increment the open-slot counter"
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

    /// Regression for PR #6592 review comment ("Saturated SSE callers can
    /// bypass request-rate protection"): a handful of capacity-rejected
    /// opens in a row (ordinary reconnect racing) stay refundable, but a
    /// caller hammering a saturated cap must eventually stop getting free
    /// 429s so `enforce_rate_limit`'s request-volume budget can still
    /// throttle them.
    #[test]
    fn repeated_rejections_stop_being_refundable_past_the_burst_limit() {
        let cap = Arc::new(SseCapacity::new(1));
        let alice = user("alice");
        let _held = cap
            .try_acquire(&tenant(), &alice, None)
            .acquired()
            .expect("first slot saturates the cap of 1");

        // The first REJECTION_REFUND_LIMIT consecutive rejections while
        // saturated are all refundable.
        for attempt in 1..=REJECTION_REFUND_LIMIT {
            let outcome = cap.try_acquire(&tenant(), &alice, None);
            assert_eq!(
                outcome.rejected_refundable(),
                Some(true),
                "attempt {attempt} is within the burst limit and must stay refundable"
            );
        }

        // Every rejection past the limit must NOT be refundable — it has
        // to drain the caller's real rate-limit budget like any other
        // request, or a saturated caller could hammer this endpoint
        // forever for free.
        for attempt in 1..=3 {
            let outcome = cap.try_acquire(&tenant(), &alice, None);
            assert_eq!(
                outcome.rejected_refundable(),
                Some(false),
                "attempt {attempt} past the burst limit must not be refundable"
            );
        }
    }

    /// Regression for the PR review finding that the `max_per_caller == 0`
    /// early return bypassed `rejected_streak` bookkeeping entirely: with a
    /// configured cap of 0 (always saturated), every rejection was reported
    /// refundable forever, so an authenticated caller could hammer SSE
    /// opens without ever draining `enforce_rate_limit`'s request-volume
    /// budget. Cap-zero must go through the same streak accounting as an
    /// ordinary saturated cap.
    #[test]
    fn zero_cap_rejections_stop_being_refundable_past_the_burst_limit() {
        let cap = Arc::new(SseCapacity::new(0));
        let alice = user("alice");

        // The first REJECTION_REFUND_LIMIT consecutive rejections while
        // saturated (cap=0 is always saturated) are all refundable.
        for attempt in 1..=REJECTION_REFUND_LIMIT {
            let outcome = cap.try_acquire(&tenant(), &alice, None);
            assert_eq!(
                outcome.rejected_refundable(),
                Some(true),
                "attempt {attempt} is within the burst limit and must stay refundable"
            );
        }

        // Every rejection past the limit must NOT be refundable — it has
        // to drain the caller's real rate-limit budget like any other
        // request, or a caller could hammer a cap-zero (SSE disabled)
        // endpoint forever for free.
        for attempt in 1..=3 {
            let outcome = cap.try_acquire(&tenant(), &alice, None);
            assert_eq!(
                outcome.rejected_refundable(),
                Some(false),
                "attempt {attempt} past the burst limit must not be refundable"
            );
        }
    }

    /// A successful acquire (the caller drops below the cap and reopens)
    /// resets the streak, so a caller who genuinely uses their slots
    /// normally is never penalized by a stale streak from an earlier,
    /// unrelated saturation episode.
    #[test]
    fn successful_acquire_resets_the_rejection_streak() {
        let cap = Arc::new(SseCapacity::new(1));
        let alice = user("alice");
        let first = cap
            .try_acquire(&tenant(), &alice, None)
            .acquired()
            .expect("first slot");

        // Burn through the whole refund budget while saturated.
        for _ in 0..=REJECTION_REFUND_LIMIT {
            cap.try_acquire(&tenant(), &alice, None);
        }
        assert_eq!(
            cap.try_acquire(&tenant(), &alice, None)
                .rejected_refundable(),
            Some(false),
            "streak must be exhausted before the reset"
        );

        // Release the slot and reacquire — the caller is no longer
        // saturated, so this succeeds and resets the streak.
        drop(first);
        let _second = cap
            .try_acquire(&tenant(), &alice, None)
            .acquired()
            .expect("slot available again after release");

        // A fresh rejection right after re-saturating must be refundable
        // again — the earlier streak must not carry over.
        assert_eq!(
            cap.try_acquire(&tenant(), &alice, None)
                .rejected_refundable(),
            Some(true),
            "a fresh saturation episode must start with a clean streak"
        );
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
            SseCapacityOutcome::Acquired(slot) => slot,
            result => panic!("first generation must be admitted: {result:?}"),
        };
        let current = match cap.try_acquire_ordered(&tenant(), &alice, Some("browser-tab"), Some(2))
        {
            SseCapacityOutcome::Acquired(slot) => slot,
            result => panic!("newer generation must be admitted: {result:?}"),
        };

        assert!(first.is_cancelled());
        assert!(!current.is_cancelled());
        assert!(matches!(
            cap.try_acquire_ordered(&tenant(), &alice, Some("browser-tab"), Some(1)),
            SseCapacityOutcome::StaleGeneration
        ));
        assert!(
            !current.is_cancelled(),
            "a late older request must not cancel the current route stream"
        );
        assert_eq!(cap.open_count(&tenant(), &alice), 1);
    }
}
