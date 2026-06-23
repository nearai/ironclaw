use ironclaw_host_api::{TenantId, UserId};
use std::collections::HashMap;
use std::num::NonZeroU32;

use crate::scope::TurnThreadOwner;

/// The origin class used for per-origin-class concurrency bucketing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum OriginClass {
    Trigger,      // TurnOriginKind::ScheduledTrigger
    Conversation, // TurnOriginKind::Inbound | TurnOriginKind::WebUi
}

/// Limits passed into the limiter. Mirrors the three concurrent-cap fields on
/// `InMemoryTurnStateStoreLimits`.
#[derive(Debug, Clone, Copy)]
pub(super) struct ConcurrencyLimits {
    pub max_concurrent_runs_per_user: Option<NonZeroU32>,
    pub max_concurrent_trigger_runs: Option<NonZeroU32>,
    pub max_concurrent_conversation_runs: Option<NonZeroU32>,
}

impl ConcurrencyLimits {
    pub(super) fn any_cap_enabled(&self) -> bool {
        self.max_concurrent_runs_per_user.is_some()
            || self.max_concurrent_trigger_runs.is_some()
            || self.max_concurrent_conversation_runs.is_some()
    }
}

/// A RunRecord-like view used by the limiter so it doesn't need to import the
/// full RunRecord (which is private to memory.rs).
pub(super) struct RunSlotInfo<'a> {
    pub tenant_id: &'a TenantId,
    pub thread_owner: &'a TurnThreadOwner,
    pub actor_user_id: &'a UserId,
    pub product_context: Option<OriginClass>,
}

/// Owns per-user and per-origin-class slot counters and all cap checks.
///
/// `Inner` holds exactly one `ConcurrencyLimiter` field instead of the two
/// scattered maps (`running_by_user`, `running_by_origin_class`) plus helpers.
#[derive(Default)]
pub(super) struct ConcurrencyLimiter {
    limits: Option<ConcurrencyLimits>,
    running_by_user: HashMap<(TenantId, UserId), u32>,
    running_by_origin_class: HashMap<(TenantId, OriginClass), u32>,
}

impl ConcurrencyLimiter {
    /// Build a limiter with no limits configured (all counters stay empty).
    #[allow(dead_code)]
    pub(super) fn unlimited() -> Self {
        Self::default()
    }

    /// Build a limiter with the given limits.  If no cap is enabled the
    /// counters are never populated (short-circuit matches original behaviour).
    pub(super) fn with_limits(limits: ConcurrencyLimits) -> Self {
        Self {
            limits: Some(limits),
            running_by_user: HashMap::new(),
            running_by_origin_class: HashMap::new(),
        }
    }

    /// Rebuild from an iterator of `RunSlotInfo` for all records whose status
    /// holds a running slot.  Skips the scan entirely when all caps are None.
    pub(super) fn rebuild_from<'a>(
        limits: ConcurrencyLimits,
        running_records: impl Iterator<Item = RunSlotInfo<'a>>,
    ) -> Self {
        if !limits.any_cap_enabled() {
            return Self::with_limits(limits);
        }
        let mut limiter = Self::with_limits(limits);
        for info in running_records {
            if let Some(key) = limiter.user_key_from(&info) {
                *limiter.running_by_user.entry(key).or_insert(0u32) += 1;
            }
            if let Some(key) = limiter.origin_key_from(&info) {
                *limiter.running_by_origin_class.entry(key).or_insert(0u32) += 1;
            }
        }
        limiter
    }

    /// Call when a run enters Running/CancelRequested.
    pub(super) fn on_enter_running(&mut self, info: RunSlotInfo<'_>) {
        if let Some(key) = self.user_key_from(&info) {
            *self.running_by_user.entry(key).or_insert(0) += 1;
        }
        if let Some(key) = self.origin_key_from(&info) {
            *self.running_by_origin_class.entry(key).or_insert(0) += 1;
        }
    }

    /// Call when a run leaves Running/CancelRequested.
    pub(super) fn on_leave_running(&mut self, info: RunSlotInfo<'_>) {
        if let Some(key) = self.user_key_from(&info)
            && let Some(count) = self.running_by_user.get_mut(&key)
        {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.running_by_user.remove(&key);
            }
        }
        if let Some(key) = self.origin_key_from(&info)
            && let Some(count) = self.running_by_origin_class.get_mut(&key)
        {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.running_by_origin_class.remove(&key);
            }
        }
    }

    /// Returns `true` if the record is allowed to claim a running slot.
    /// Ownerless runs are always allowed; runs without product_context are
    /// origin-uncapped.
    pub(super) fn can_claim(&self, info: &RunSlotInfo<'_>) -> bool {
        let user_ok = match self
            .limits
            .as_ref()
            .and_then(|l| l.max_concurrent_runs_per_user)
        {
            None => true,
            Some(cap) => match self.user_key_from(info) {
                None => true, // ownerless
                Some(key) => self.running_by_user.get(&key).copied().unwrap_or(0) < cap.get(),
            },
        };
        if !user_ok {
            return false;
        }
        match info.product_context {
            None => true,
            Some(OriginClass::Trigger) => self
                .limits
                .as_ref()
                .and_then(|l| l.max_concurrent_trigger_runs)
                .is_none_or(|cap| {
                    let key = (info.tenant_id.clone(), OriginClass::Trigger);
                    self.running_by_origin_class.get(&key).copied().unwrap_or(0) < cap.get()
                }),
            Some(OriginClass::Conversation) => self
                .limits
                .as_ref()
                .and_then(|l| l.max_concurrent_conversation_runs)
                .is_none_or(|cap| {
                    let key = (info.tenant_id.clone(), OriginClass::Conversation);
                    self.running_by_origin_class.get(&key).copied().unwrap_or(0) < cap.get()
                }),
        }
    }

    // ── Observability accessors (same public contract as old Inner methods) ──

    pub(super) fn count_for_user(&self, tenant: &TenantId, user: &UserId) -> u32 {
        self.running_by_user
            .get(&(tenant.clone(), user.clone()))
            .copied()
            .unwrap_or(0)
    }

    pub(super) fn count_for_trigger(&self, tenant: &TenantId) -> u32 {
        self.running_by_origin_class
            .get(&(tenant.clone(), OriginClass::Trigger))
            .copied()
            .unwrap_or(0)
    }

    pub(super) fn count_for_conversation(&self, tenant: &TenantId) -> u32 {
        self.running_by_origin_class
            .get(&(tenant.clone(), OriginClass::Conversation))
            .copied()
            .unwrap_or(0)
    }

    // ── Private key derivation (was `Inner::run_user_key` / `run_origin_key`) ──

    fn user_key_from(&self, info: &RunSlotInfo<'_>) -> Option<(TenantId, UserId)> {
        match info.thread_owner {
            TurnThreadOwner::ExplicitUser { owner_user_id } => {
                Some((info.tenant_id.clone(), owner_user_id.clone()))
            }
            TurnThreadOwner::ActorFallback => {
                Some((info.tenant_id.clone(), info.actor_user_id.clone()))
            }
            TurnThreadOwner::Ownerless => None,
        }
    }

    fn origin_key_from(&self, info: &RunSlotInfo<'_>) -> Option<(TenantId, OriginClass)> {
        info.product_context
            .map(|cls| (info.tenant_id.clone(), cls))
    }
}
