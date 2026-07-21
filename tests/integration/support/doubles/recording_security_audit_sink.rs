/// Test double substituting production's `TracingSecurityAuditSink`.
use std::sync::Mutex;

use ironclaw_events::{SecurityAuditEvent, SecurityAuditSink};

#[derive(Debug, Default)]
pub(crate) struct RecordingSecurityAuditSink {
    events: Mutex<Vec<SecurityAuditEvent>>,
}

impl RecordingSecurityAuditSink {
    pub(crate) fn events(&self) -> Vec<SecurityAuditEvent> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl SecurityAuditSink for RecordingSecurityAuditSink {
    fn record(&self, event: SecurityAuditEvent) {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}
