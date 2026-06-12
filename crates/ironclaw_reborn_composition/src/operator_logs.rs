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
const MAX_LOG_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_LOG_RESPONSE_BYTES: usize = 256 * 1024;
const LOG_TRUNCATED_SUFFIX: &str = " ... [truncated]";
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
        let message = truncate_utf8_with_suffix(&message, MAX_LOG_MESSAGE_BYTES);
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
        let Ok(state) = self.state.lock() else {
            return RebornLogQueryResponse {
                source: SOURCE.to_string(),
                entries: Vec::new(),
                next_cursor: None,
                tail_supported: true,
                follow_supported: false,
            };
        };

        let mut selected = Vec::with_capacity(limit.min(self.capacity));
        let mut selected_bytes = 0usize;
        let mut next_cursor = None;
        for entry in state.entries.iter().rev() {
            if before_id.is_some_and(|id| entry.id >= id) {
                continue;
            }
            if request.level.is_some_and(|level| entry.level != level) {
                continue;
            }
            if let Some(target) = target_filter.as_ref()
                && !entry.target.to_lowercase().contains(target.as_str())
            {
                continue;
            }

            if selected.len() >= limit {
                next_cursor = selected
                    .last()
                    .map(|entry: &StoredLogEntry| format!("before:{}", entry.id));
                break;
            }
            let entry_bytes = response_entry_bytes(entry);
            if !selected.is_empty()
                && selected_bytes.saturating_add(entry_bytes) > MAX_LOG_RESPONSE_BYTES
            {
                next_cursor = selected
                    .last()
                    .map(|entry: &StoredLogEntry| format!("before:{}", entry.id));
                break;
            }
            selected_bytes = selected_bytes.saturating_add(entry_bytes);
            selected.push(entry.clone());
        }

        RebornLogQueryResponse {
            source: SOURCE.to_string(),
            entries: selected.into_iter().map(RebornLogEntry::from).collect(),
            next_cursor,
            tail_supported: true,
            follow_supported: false,
        }
    }
}

fn response_entry_bytes(entry: &StoredLogEntry) -> usize {
    entry.id.to_string().len()
        + entry.timestamp.to_rfc3339().len()
        + entry.target.len()
        + entry.message.len()
        + 64
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

fn truncate_utf8_with_suffix(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }

    if max_bytes <= LOG_TRUNCATED_SUFFIX.len() {
        return LOG_TRUNCATED_SUFFIX[..max_bytes].to_string();
    }

    let mut end = max_bytes - LOG_TRUNCATED_SUFFIX.len();
    while !value.is_char_boundary(end) {
        end -= 1;
    }

    let mut truncated = String::with_capacity(max_bytes);
    truncated.push_str(&value[..end]);
    truncated.push_str(LOG_TRUNCATED_SUFFIX);
    truncated
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

    #[test]
    fn record_truncates_large_messages_on_utf8_boundary() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(
            RebornLogLevel::Info,
            "ironclaw::test",
            "\u{1F600}".repeat(MAX_LOG_MESSAGE_BYTES),
        );

        let response = buffer.query(RebornLogQueryRequest {
            limit: Some(1),
            ..RebornLogQueryRequest::default()
        });

        let message = &response.entries[0].message;
        assert!(message.len() <= MAX_LOG_MESSAGE_BYTES);
        assert!(message.ends_with(LOG_TRUNCATED_SUFFIX));
        assert!(message.is_char_boundary(message.len()));
    }

    #[test]
    fn query_enforces_response_byte_budget() {
        let buffer = OperatorLogBuffer::new(100);
        for index in 0..40 {
            buffer.record(
                RebornLogLevel::Info,
                "ironclaw::test",
                format!("{index}:{}", "x".repeat(MAX_LOG_MESSAGE_BYTES)),
            );
        }

        let response = buffer.query(RebornLogQueryRequest {
            limit: Some(100),
            ..RebornLogQueryRequest::default()
        });

        let message_bytes = response
            .entries
            .iter()
            .map(|entry| entry.message.len())
            .sum::<usize>();
        assert!(message_bytes <= MAX_LOG_RESPONSE_BYTES);
        assert!(response.entries.len() < 40);
        assert!(response.next_cursor.is_some());
    }
}
