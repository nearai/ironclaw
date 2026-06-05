//! Tracing layer that broadcasts log events to the web gateway via SSE.
//!
//! ```text
//! tracing::info!("...")
//!        │
//!        ▼
//!   WebLogLayer::on_event()
//!        │
//!        ▼
//!   LogBroadcaster::send()
//!        │
//!        ├──► broadcast::Sender<LogEntry>  (live SSE subscribers)
//!        └──► mpsc::Sender<LogEntry>       (DB writer, info+ only)
//!                   │
//!                   ▼
//!             log_entries table  ←  queried on GET /api/logs/events
//! ```

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::{broadcast, mpsc};
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, reload};

use t3claw_safety::LeakDetector;

/// A single log entry broadcast to connected clients.
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub level: String,
    pub target: String,
    pub message: String,
    pub timestamp: String,
}

/// Broadcasts log entries to SSE subscribers and an optional DB writer.
///
/// Created early in startup (before the DB is available). Call
/// `set_db_writer` once the DB is initialised to start persisting
/// `info` and above.
pub struct LogBroadcaster {
    tx: broadcast::Sender<LogEntry>,
    /// Set after DB init; receives info+ entries for async persistence.
    db_writer: Mutex<Option<mpsc::Sender<LogEntry>>>,
    /// Scrubs secrets from log messages before broadcasting to SSE clients.
    leak_detector: LeakDetector,
}

impl LogBroadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(512);
        Self {
            tx,
            db_writer: Mutex::new(None),
            leak_detector: LeakDetector::new(),
        }
    }

    /// Wire up the background DB writer. Call once after DB init.
    pub fn set_db_writer(&self, sender: mpsc::Sender<LogEntry>) {
        if let Ok(mut w) = self.db_writer.lock() {
            *w = Some(sender);
        }
    }

    pub fn send(&self, mut entry: LogEntry) {
        // Scrub secrets before anything reaches subscribers or the DB.
        entry.message = self
            .leak_detector
            .scan_and_clean(&entry.message)
            .unwrap_or_else(|_| "[log message redacted: contained blocked secret]".to_string());

        // Persist info+ to DB (fire-and-forget; drops on full channel).
        if entry.level != "DEBUG" && entry.level != "TRACE" {
            if let Ok(guard) = self.db_writer.lock() {
                if let Some(ref tx) = *guard {
                    let _ = tx.try_send(entry.clone());
                }
            }
        }

        // Broadcast to live SSE subscribers.
        let _ = self.tx.send(entry);
    }

    /// Subscribe to the live event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.tx.subscribe()
    }
}

impl Default for LogBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle for changing the tracing `EnvFilter` at runtime.
///
/// Wraps a `reload::Handle` so the gateway can switch between log levels
/// (e.g. `t3claw=debug`) without restarting the process.
pub struct LogLevelHandle {
    handle: reload::Handle<EnvFilter, tracing_subscriber::Registry>,
    current_level: Mutex<String>,
    base_filter: String,
}

impl LogLevelHandle {
    pub fn new(
        handle: reload::Handle<EnvFilter, tracing_subscriber::Registry>,
        initial_level: String,
        base_filter: String,
    ) -> Self {
        Self {
            handle,
            current_level: Mutex::new(initial_level),
            base_filter,
        }
    }

    /// Change the `t3claw=<level>` directive at runtime.
    ///
    /// `level` must be one of: trace, debug, info, warn, error.
    pub fn set_level(&self, level: &str) -> Result<(), String> {
        const VALID: &[&str] = &["trace", "debug", "info", "warn", "error"];
        let level = level.to_lowercase();
        if !VALID.contains(&level.as_str()) {
            return Err(format!(
                "invalid level '{}', must be one of: {}",
                level,
                VALID.join(", ")
            ));
        }

        let filter_str = if self.base_filter.is_empty() {
            format!("t3claw={}", level)
        } else {
            format!("t3claw={},{}", level, self.base_filter)
        };

        let new_filter = EnvFilter::new(&filter_str);
        self.handle
            .reload(new_filter)
            .map_err(|e| format!("failed to reload filter: {}", e))?;

        if let Ok(mut current) = self.current_level.lock() {
            *current = level;
        }
        Ok(())
    }

    /// Returns the current t3claw log level (e.g. "info", "debug").
    pub fn current_level(&self) -> String {
        self.current_level
            .lock()
            .map(|l| l.clone())
            .unwrap_or_else(|_| "info".to_string())
    }
}

/// Initialise the tracing subscriber with a reloadable `EnvFilter`.
///
/// Returns the `LogLevelHandle` so callers can swap the filter at runtime.
/// The fmt layer and `WebLogLayer` are attached alongside the reloadable filter.
///
/// When `suppress_stderr` is true, the stderr formatter is omitted. This is
/// used in TUI mode where logs are displayed in the dedicated Logs tab instead
/// of interleaving with the alternate screen.
pub fn init_tracing(
    log_broadcaster: Arc<LogBroadcaster>,
    suppress_stderr: bool,
) -> Arc<LogLevelHandle> {
    let raw_filter =
        std::env::var("RUST_LOG").unwrap_or_else(|_| "t3claw=info,tower_http=warn".to_string());

    // Split into the t3claw directive and "everything else" (base_filter).
    let mut t3claw_level = String::from("info");
    let mut base_parts: Vec<&str> = Vec::new();

    for part in raw_filter.split(',') {
        let trimmed = part.trim();
        if trimmed.starts_with("t3claw=") {
            if let Some(lvl) = trimmed.strip_prefix("t3claw=") {
                t3claw_level = lvl.to_string();
            }
        } else if !trimmed.is_empty() {
            base_parts.push(trimmed);
        }
    }
    let base_filter = base_parts.join(",");

    let env_filter = EnvFilter::new(&raw_filter);
    let (reload_layer, reload_handle) = reload::Layer::new(env_filter);

    let handle = Arc::new(LogLevelHandle::new(
        reload_handle,
        t3claw_level,
        base_filter,
    ));

    let fmt_layer = if suppress_stderr {
        None
    } else {
        Some(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_writer(crate::tracing_fmt::TruncatingStderr::default()),
        )
    };

    tracing_subscriber::registry()
        .with(reload_layer)
        .with(fmt_layer)
        .with(WebLogLayer::new(log_broadcaster))
        .init();

    handle
}

/// Visitor that extracts the `message` field and all extra key-value
/// fields from a tracing event.
///
/// The terminal formatter shows something like:
///   INFO t3claw::agent: Request completed url="http://..." status=200
///
/// We replicate that by capturing both the message and the extra fields.
struct MessageVisitor {
    message: String,
    fields: Vec<String>,
}

impl MessageVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
            fields: Vec::new(),
        }
    }

    /// Build the final message string: "message key=val key=val ..."
    fn finish(self) -> String {
        if self.fields.is_empty() {
            self.message
        } else {
            format!("{} {}", self.message, self.fields.join(" "))
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
            // Strip surrounding quotes from Debug output
            if self.message.starts_with('"') && self.message.ends_with('"') {
                self.message = self.message[1..self.message.len() - 1].to_string();
            }
        } else {
            self.fields.push(format!("{}={:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push(format!("{}={}", field.name(), value));
        }
    }
}

/// Tracing layer that forwards events to a [`LogBroadcaster`].
///
/// Only forwards DEBUG and above. Attach to the tracing subscriber
/// alongside the existing fmt layer.
///
/// Log messages are scrubbed through `LeakDetector` in `LogBroadcaster::send()`
/// (the single funnel point for all log output, including late-joiner history).
pub struct WebLogLayer {
    broadcaster: Arc<LogBroadcaster>,
}

impl WebLogLayer {
    pub fn new(broadcaster: Arc<LogBroadcaster>) -> Self {
        Self { broadcaster }
    }
}

impl<S: tracing::Subscriber> Layer<S> for WebLogLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let metadata = event.metadata();

        // Only forward DEBUG+
        if *metadata.level() > tracing::Level::DEBUG {
            return;
        }

        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);

        let entry = LogEntry {
            level: metadata.level().to_string().to_uppercase(),
            target: metadata.target().to_string(),
            message: visitor.finish(),
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };

        // LeakDetector scrubbing happens inside broadcaster.send()
        self.broadcaster.send(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_broadcaster_creation() {
        let broadcaster = LogBroadcaster::new();
        // Should not panic with no receivers
        broadcaster.send(LogEntry {
            level: "INFO".to_string(),
            target: "test".to_string(),
            message: "hello".to_string(),
            timestamp: "2024-01-01T00:00:00.000Z".to_string(),
        });
    }

    #[test]
    fn test_log_broadcaster_subscribe() {
        let broadcaster = LogBroadcaster::new();
        let mut rx = broadcaster.subscribe();

        broadcaster.send(LogEntry {
            level: "WARN".to_string(),
            target: "t3claw::test".to_string(),
            message: "test warning".to_string(),
            timestamp: "2024-01-01T00:00:00.000Z".to_string(),
        });

        let entry = rx.try_recv().expect("should receive entry");
        assert_eq!(entry.level, "WARN");
        assert_eq!(entry.message, "test warning");
    }

    #[test]
    fn test_log_entry_serialization() {
        let entry = LogEntry {
            level: "ERROR".to_string(),
            target: "t3claw::agent".to_string(),
            message: "something broke".to_string(),
            timestamp: "2024-01-01T00:00:00.000Z".to_string(),
        };
        let json = serde_json::to_string(&entry).expect("should serialize");
        assert!(json.contains("\"level\":\"ERROR\""));
        assert!(json.contains("something broke"));
    }

    #[test]
    fn test_message_visitor_finish_message_only() {
        let v = MessageVisitor {
            message: "hello world".to_string(),
            fields: vec![],
        };
        assert_eq!(v.finish(), "hello world");
    }

    #[test]
    fn test_message_visitor_finish_with_fields() {
        let v = MessageVisitor {
            message: "Request completed".to_string(),
            fields: vec![
                "url=http://localhost:8080".to_string(),
                "status=200".to_string(),
            ],
        };
        let result = v.finish();
        assert_eq!(
            result,
            "Request completed url=http://localhost:8080 status=200"
        );
    }

    #[test]
    fn test_message_visitor_finish_empty() {
        let v = MessageVisitor::new();
        assert_eq!(v.finish(), "");
    }

    #[test]
    fn test_broadcaster_has_leak_detector() {
        let broadcaster = LogBroadcaster::new();
        // Verify the leak detector is initialized with default patterns
        assert!(broadcaster.leak_detector.pattern_count() > 0);
    }

    #[test]
    fn test_leak_detector_scrubs_api_key_in_log() {
        let detector = t3claw_safety::LeakDetector::new();
        let msg = "Connecting with token sk-proj-test1234567890abcdefghij";
        let result = detector.scan_and_clean(msg);
        // Should be blocked (OpenAI key pattern)
        assert!(result.is_err());
    }

    #[test]
    fn test_leak_detector_passes_clean_log() {
        let detector = t3claw_safety::LeakDetector::new();
        let msg = "Request completed status=200 url=https://api.example.com/data";
        let result = detector.scan_and_clean(msg);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), msg);
    }
}
