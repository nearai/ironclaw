//! Context for event-triggered hooks.
//!
//! Event-triggered hooks observe durable runtime facts after the originating
//! loop work has already happened. They therefore get read-only event context
//! plus an observer-only sink; they cannot gate, patch, or retroactively alter
//! the completed behavior.
//!
//! # Field exposure
//!
//! `event` is the full [`RuntimeEvent`], including its `ResourceScope`
//! (tenant_id, user_id, agent_id, project_id, mission_id, thread_id,
//! invocation_id). For Installed-tier hooks this is more identifying
//! information than the trust class warrants. The longer-term plan is to
//! hand Installed hooks a narrowed `HookObservableEvent` projection that
//! strips dispatcher-internal scope fields; trusted tiers may continue to
//! receive the full event. Tracked as issue #3690.

use ironclaw_events::{EventCursor, RuntimeEvent};
use ironclaw_host_api::TenantId;

/// Read-only context handed to an event-triggered hook.
///
/// `is_replay` is `true` when the dispatcher is replaying a previously
/// observed event after a host restart. Subscriptions are at-least-once
/// (see [`crate::sink::EventTriggeredHook`] docs), so a hook that performs
/// side effects through its observer sink should treat `is_replay` as a
/// signal to dedupe by `event.event_id` instead of re-firing notifications
/// (PR #3640 finding A3).
#[derive(Debug)]
#[non_exhaustive]
pub struct EventTriggeredHookContext<'a> {
    pub tenant_id: TenantId,
    pub event: &'a RuntimeEvent,
    pub event_cursor: EventCursor,
    pub is_replay: bool,
}
