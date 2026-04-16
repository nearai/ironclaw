//! Unified runtime log contract for IronClaw.
//!
//! Defines stable field names and helpers used across all runtime boundaries
//! (gateway, webhook, WebSocket, worker, job, agent) so that logs are
//! consistently structured and can be correlated by field name in both
//! local viewers (SSE / TUI / stderr) and platform-level persistence
//! (ClickHouse `platform_logs`).
//!
//! ## Field Contract
//!
//! | Field | Type | Source | Description |
//! |-------|------|--------|-------------|
//! | `request_id` | String | Gateway / webhook / WS | Per-request correlation key |
//! | `channel` | String | Gateway / agent loop | Channel name (gateway, http, dingtalk, signal, …) |
//! | `thread_id` | String | Session manager | Conversation thread ID |
//! | `job_id` | String | Scheduler / orchestrator | Background job ID |
//! | `session_id` | String | Session manager | User session ID |
//! | `phase` | String | Lifecycle code | Lifecycle phase (e.g. "start", "complete", "fail") |
//! | `component` | String | Module code | Subsystem name (e.g. "gateway", "worker", "agent") |
//! | `tenant_id` | String | Platform injection | Tenant ID (platform-managed only) |
//! | `agent_id` | String | Platform injection | Agent ID (platform-managed only) |

/// Stable field name constants.
///
/// Using constants prevents typos and makes `grep` reliable across the codebase.
pub mod fields {
    pub const REQUEST_ID: &str = "request_id";
    pub const CHANNEL: &str = "channel";
    pub const THREAD_ID: &str = "thread_id";
    pub const JOB_ID: &str = "job_id";
    pub const SESSION_ID: &str = "session_id";
    pub const PHASE: &str = "phase";
    pub const COMPONENT: &str = "component";
    pub const TENANT_ID: &str = "tenant_id";
    pub const AGENT_ID: &str = "agent_id";
}

/// Well-known phase values for lifecycle events.
pub mod phases {
    pub const START: &str = "start";
    pub const COMPLETE: &str = "complete";
    pub const FAIL: &str = "fail";
    pub const TIMEOUT: &str = "timeout";
    pub const REJECT: &str = "reject";
    pub const ACCEPT: &str = "accept";
    pub const PAUSE: &str = "pause";
    pub const RESUME: &str = "resume";
    pub const REPAIR: &str = "repair";
    pub const SCHEDULE: &str = "schedule";
    pub const PENDING: &str = "pending";
    pub const CONNECT: &str = "connect";
    pub const DISCONNECT: &str = "disconnect";
}

/// Well-known component values.
pub mod components {
    pub const GATEWAY: &str = "gateway";
    pub const WEBHOOK: &str = "webhook";
    pub const WEBSOCKET: &str = "websocket";
    pub const WORKER: &str = "worker";
    pub const JOB: &str = "job";
    pub const AGENT: &str = "agent";
    pub const SCHEDULER: &str = "scheduler";
    pub const ORCHESTRATOR: &str = "orchestrator";
    pub const PLATFORM_SINK: &str = "platform_sink";
}

/// A set of well-known context fields extracted from span scope.
///
/// Used by `WebLogLayer` and the platform ClickHouse sink to propagate
/// context from enclosing spans into log entries without requiring every
/// call site to repeat field names.
#[derive(Debug, Default, Clone)]
pub struct SpanContext {
    pub request_id: Option<String>,
    pub channel: Option<String>,
    pub thread_id: Option<String>,
    pub job_id: Option<String>,
    pub session_id: Option<String>,
    pub tenant_id: Option<String>,
    pub agent_id: Option<String>,
}

impl SpanContext {
    /// Check if any context field is populated.
    pub fn has_any(&self) -> bool {
        self.request_id.is_some()
            || self.channel.is_some()
            || self.thread_id.is_some()
            || self.job_id.is_some()
            || self.session_id.is_some()
            || self.tenant_id.is_some()
            || self.agent_id.is_some()
    }

    /// Merge another context into this one without overwriting fields that
    /// are already present.
    pub fn merge_missing_from(&mut self, other: SpanContext) {
        if self.request_id.is_none() {
            self.request_id = other.request_id;
        }
        if self.channel.is_none() {
            self.channel = other.channel;
        }
        if self.thread_id.is_none() {
            self.thread_id = other.thread_id;
        }
        if self.job_id.is_none() {
            self.job_id = other.job_id;
        }
        if self.session_id.is_none() {
            self.session_id = other.session_id;
        }
        if self.tenant_id.is_none() {
            self.tenant_id = other.tenant_id;
        }
        if self.agent_id.is_none() {
            self.agent_id = other.agent_id;
        }
    }
}

/// Visitor that extracts well-known context fields from a span's attributes
/// or recorded values.
pub(crate) struct SpanContextVisitor<'a>(pub &'a mut SpanContext);

impl<'a> tracing::field::Visit for SpanContextVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let val = format!("{:?}", value);
        // Strip surrounding quotes from Debug output
        let val = val
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(&val)
            .to_string();
        self.record_str_inner(field.name(), &val);
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record_str_inner(field.name(), value);
    }
}

impl<'a> SpanContextVisitor<'a> {
    fn record_str_inner(&mut self, name: &str, value: &str) {
        if value.is_empty() {
            return;
        }
        match name {
            fields::REQUEST_ID => self.0.request_id = Some(value.to_string()),
            fields::CHANNEL => self.0.channel = Some(value.to_string()),
            fields::THREAD_ID => self.0.thread_id = Some(value.to_string()),
            fields::JOB_ID => self.0.job_id = Some(value.to_string()),
            fields::SESSION_ID => self.0.session_id = Some(value.to_string()),
            fields::TENANT_ID => self.0.tenant_id = Some(value.to_string()),
            fields::AGENT_ID => self.0.agent_id = Some(value.to_string()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_context_has_any_empty() {
        let ctx = SpanContext::default();
        assert!(!ctx.has_any());
    }

    #[test]
    fn span_context_has_any_with_request_id() {
        let ctx = SpanContext {
            request_id: Some("abc".to_string()),
            ..Default::default()
        };
        assert!(ctx.has_any());
    }

    #[test]
    fn span_context_has_any_with_job_id() {
        let ctx = SpanContext {
            job_id: Some("123".to_string()),
            ..Default::default()
        };
        assert!(ctx.has_any());
    }

    #[test]
    fn field_constants_are_stable() {
        assert_eq!(fields::REQUEST_ID, "request_id");
        assert_eq!(fields::CHANNEL, "channel");
        assert_eq!(fields::THREAD_ID, "thread_id");
        assert_eq!(fields::JOB_ID, "job_id");
        assert_eq!(fields::SESSION_ID, "session_id");
        assert_eq!(fields::PHASE, "phase");
        assert_eq!(fields::COMPONENT, "component");
        assert_eq!(fields::TENANT_ID, "tenant_id");
        assert_eq!(fields::AGENT_ID, "agent_id");
    }

    #[test]
    fn phase_constants_are_stable() {
        assert_eq!(phases::START, "start");
        assert_eq!(phases::COMPLETE, "complete");
        assert_eq!(phases::FAIL, "fail");
        assert_eq!(phases::TIMEOUT, "timeout");
    }

    #[test]
    fn component_constants_are_stable() {
        assert_eq!(components::GATEWAY, "gateway");
        assert_eq!(components::WORKER, "worker");
        assert_eq!(components::AGENT, "agent");
    }

    #[test]
    fn merge_missing_from_keeps_existing_values() {
        let mut base = SpanContext {
            request_id: Some("req-1".to_string()),
            tenant_id: Some("tenant-a".to_string()),
            ..Default::default()
        };

        let incoming = SpanContext {
            request_id: Some("req-2".to_string()),
            channel: Some("gateway".to_string()),
            tenant_id: Some("tenant-b".to_string()),
            agent_id: Some("agent-1".to_string()),
            ..Default::default()
        };

        base.merge_missing_from(incoming);

        assert_eq!(base.request_id.as_deref(), Some("req-1"));
        assert_eq!(base.channel.as_deref(), Some("gateway"));
        assert_eq!(base.tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(base.agent_id.as_deref(), Some("agent-1"));
    }
}
