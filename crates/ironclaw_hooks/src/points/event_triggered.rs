//! Context for event-triggered hooks.
//!
//! Event-triggered hooks observe durable runtime facts after the originating
//! loop work has already happened. They therefore get read-only event context
//! plus an observer-only sink; they cannot gate, patch, or retroactively alter
//! the completed behavior.

use ironclaw_events::{EventCursor, RuntimeEvent};
use ironclaw_host_api::TenantId;

/// Read-only context handed to an event-triggered hook.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EventTriggeredHookContext<'a> {
    pub tenant_id: TenantId,
    pub event: &'a RuntimeEvent,
    pub event_cursor: EventCursor,
}
