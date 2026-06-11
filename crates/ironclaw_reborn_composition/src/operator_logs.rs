use std::collections::VecDeque;
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_product_workflow::{
    OperatorLogsService, RebornLogEntry, RebornLogLevel, RebornLogQueryRequest,
    RebornLogQueryResponse, RebornServicesError, WebUiAuthenticatedCaller,
};
use ironclaw_safety::LeakDetector;

const HISTORY_CAP: usize = 500;
const DEFAULT_LIMIT: usize = 100;
const SOURCE: &str = "in_memory_tracing";

static OPERATOR_LOGS: LazyLock<Arc<OperatorLogBuffer>> =
    LazyLock::new(|| Arc::new(OperatorLogBuffer::new(HISTORY_CAP)));

#[derive(Debug, Clone)]
struct StoredLogEntry {
    id: u64,
    timestamp: DateTime<Utc>,
    level: RebornLogLevel,
    target: String,
    message: String,
}

#[derive(Debug)]
struct OperatorLogState {
    next_id: u64,
    entries: VecDeque<StoredLogEntry>,
}

pub struct OperatorLogBuffer {
    capacity: usize,
    state: Mutex<OperatorLogState>,
    leak_detector: LeakDetector,
}

impl OperatorLogBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            state: Mutex::new(OperatorLogState {
                next_id: 1,
                entries: VecDeque::with_capacity(capacity),
            }),
            leak_detector: LeakDetector::new(),
        }
    }

    pub fn record(&self, level: RebornLogLevel, target: &str, message: String) {
        let message = self
            .leak_detector
            .scan_and_clean(&message)
            .unwrap_or_else(|_| "[log message redacted: contained blocked secret]".to_string());
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let id = state.next_id;
        state.next_id = state.next_id.saturating_add(1);
        if state.entries.len() >= self.capacity {
            state.entries.pop_front();
        }
        state.entries.push_back(StoredLogEntry {
            id,
            timestamp: Utc::now(),
            level,
            target: target.to_string(),
            message,
        });
    }

    fn query(&self, request: RebornLogQueryRequest) -> RebornLogQueryResponse {
        let limit = request
            .limit
            .map(|value| value as usize)
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(1, self.capacity);
        let before_id = request.cursor.as_deref().and_then(parse_before_cursor);
        let target_filter = request.target.map(|target| target.to_lowercase());
        let entries = self
            .state
            .lock()
            .map(|state| state.entries.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        let mut selected = entries
            .into_iter()
            .rev()
            .filter(|entry| before_id.is_none_or(|id| entry.id < id))
            .filter(|entry| request.level.is_none_or(|level| entry.level == level))
            .filter(|entry| {
                target_filter
                    .as_ref()
                    .is_none_or(|target| entry.target.to_lowercase().contains(target.as_str()))
            })
            .take(limit + 1)
            .collect::<Vec<_>>();

        let next_cursor = if selected.len() > limit {
            selected.truncate(limit);
            selected.last().map(|entry| format!("before:{}", entry.id))
        } else {
            None
        };

        RebornLogQueryResponse {
            source: SOURCE.to_string(),
            entries: selected.into_iter().map(RebornLogEntry::from).collect(),
            next_cursor,
            tail_supported: true,
            follow_supported: false,
        }
    }
}

impl From<StoredLogEntry> for RebornLogEntry {
    fn from(entry: StoredLogEntry) -> Self {
        Self {
            id: entry.id.to_string(),
            timestamp: entry.timestamp,
            level: entry.level,
            target: entry.target,
            message: entry.message,
        }
    }
}

#[async_trait]
impl OperatorLogsService for OperatorLogBuffer {
    async fn query_logs(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornLogQueryRequest,
    ) -> Result<RebornLogQueryResponse, RebornServicesError> {
        Ok(self.query(request))
    }
}

pub fn operator_log_buffer() -> Arc<OperatorLogBuffer> {
    Arc::clone(&OPERATOR_LOGS)
}

pub fn capture_tracing_log(level: &tracing::Level, target: &str, message: String) {
    operator_log_buffer().record(reborn_level_from_tracing(level), target, message);
}

fn reborn_level_from_tracing(level: &tracing::Level) -> RebornLogLevel {
    match *level {
        tracing::Level::TRACE => RebornLogLevel::Trace,
        tracing::Level::DEBUG => RebornLogLevel::Debug,
        tracing::Level::INFO => RebornLogLevel::Info,
        tracing::Level::WARN => RebornLogLevel::Warn,
        tracing::Level::ERROR => RebornLogLevel::Error,
    }
}

fn parse_before_cursor(cursor: &str) -> Option<u64> {
    cursor
        .strip_prefix("before:")
        .and_then(|value| value.parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_returns_newest_first_and_paginates() {
        let buffer = OperatorLogBuffer::new(10);
        for index in 0..5 {
            buffer.record(
                RebornLogLevel::Info,
                "ironclaw::test",
                format!("message {index}"),
            );
        }

        let first = buffer.query(RebornLogQueryRequest {
            limit: Some(2),
            ..RebornLogQueryRequest::default()
        });
        assert_eq!(
            first
                .entries
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>(),
            vec!["message 4", "message 3"]
        );
        let cursor = first.next_cursor.expect("older page cursor");

        let second = buffer.query(RebornLogQueryRequest {
            limit: Some(2),
            cursor: Some(cursor),
            ..RebornLogQueryRequest::default()
        });
        assert_eq!(
            second
                .entries
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>(),
            vec!["message 2", "message 1"]
        );
    }

    #[test]
    fn query_filters_by_level_and_target() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(RebornLogLevel::Info, "ironclaw::alpha", "alpha".to_string());
        buffer.record(RebornLogLevel::Warn, "ironclaw::beta", "beta".to_string());
        buffer.record(RebornLogLevel::Warn, "other::beta", "other".to_string());

        let response = buffer.query(RebornLogQueryRequest {
            limit: Some(10),
            level: Some(RebornLogLevel::Warn),
            target: Some("ironclaw".to_string()),
            ..RebornLogQueryRequest::default()
        });

        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].message, "beta");
    }

    #[test]
    fn record_redacts_secret_shaped_messages() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(
            RebornLogLevel::Info,
            "ironclaw::test",
            "token sk-proj-test1234567890abcdefghij".to_string(),
        );

        let response = buffer.query(RebornLogQueryRequest {
            limit: Some(1),
            ..RebornLogQueryRequest::default()
        });

        assert_eq!(
            response.entries[0].message,
            "[log message redacted: contained blocked secret]"
        );
    }
}
