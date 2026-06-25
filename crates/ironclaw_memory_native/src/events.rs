//! Metadata-only memory significant-event seam.
//!
//! The event vocabulary (`MemorySignificantEvent`, its kinds/sources/status,
//! `MemoryAuditContext`, `MemoryEventSinkError`, and the
//! `MemorySignificantEventSink` trait) moved to `ironclaw_memory` and is
//! re-exported below. The `pub(crate)` host-composed logging helper stays here
//! because it depends on `tracing`.

use std::sync::Arc;

pub use ironclaw_memory::{
    MemoryAuditContext, MemoryEventSinkError, MemorySignificantEvent, MemorySignificantEventKind,
    MemorySignificantEventSink, MemorySignificantEventSource, MemorySignificantEventStatus,
};

pub(crate) async fn record_memory_significant_event(
    sink: Option<&Arc<dyn MemorySignificantEventSink>>,
    event: MemorySignificantEvent,
) {
    let Some(sink) = sink else {
        return;
    };
    if let Err(error) = sink.record_memory_significant_event(event).await {
        tracing::debug!(error = %error, "memory significant-event sink failed");
    }
}
