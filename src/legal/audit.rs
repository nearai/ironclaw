use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::LegalAuditConfig;

#[derive(Debug, Default, Clone, Serialize)]
struct SecurityMetrics {
    blocked_actions: u64,
    approval_required: u64,
    redaction_events: u64,
}

#[derive(Debug, Serialize)]
struct AuditEvent<'a> {
    ts: String,
    event_type: &'a str,
    details: serde_json::Value,
    metrics: SecurityMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    prev_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hash: Option<String>,
}

struct AuditLogger {
    path: PathBuf,
    hash_chain: bool,
    state: Mutex<Option<String>>,
    metrics: Mutex<SecurityMetrics>,
}

impl AuditLogger {
    fn new(path: PathBuf, hash_chain: bool) -> Self {
        Self {
            path,
            hash_chain,
            state: Mutex::new(None),
            metrics: Mutex::new(SecurityMetrics::default()),
        }
    }

    fn bump_metric<F>(&self, update: F)
    where
        F: FnOnce(&mut SecurityMetrics),
    {
        if let Ok(mut metrics) = self.metrics.lock() {
            update(&mut metrics);
        }
    }

    fn write(&self, event_type: &str, details: serde_json::Value) {
        let metrics = self
            .metrics
            .lock()
            .map(|m| m.clone())
            .unwrap_or_else(|_| SecurityMetrics::default());

        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Legal audit state lock poisoned: {}", e);
                return;
            }
        };

        let prev_hash = state.clone();
        let mut event = AuditEvent {
            ts: Utc::now().to_rfc3339(),
            event_type,
            details,
            metrics,
            prev_hash,
            hash: None,
        };

        if self.hash_chain {
            let to_hash = match serde_json::to_string(&event) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to serialize audit event for hashing: {}", e);
                    return;
                }
            };
            let mut hasher = Sha256::new();
            hasher.update(to_hash.as_bytes());
            let hash = format!("{:x}", hasher.finalize());
            event.hash = Some(hash.clone());
            *state = Some(hash);
        }

        let line = match serde_json::to_string(&event) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to serialize legal audit event: {}", e);
                return;
            }
        };

        if let Some(parent) = self.path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::warn!("Failed to create legal audit log dir {:?}: {}", parent, e);
            return;
        }

        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(mut f) => {
                if let Err(e) = writeln!(f, "{line}") {
                    tracing::warn!("Failed to append legal audit event: {}", e);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to open legal audit log {:?}: {}", self.path, e);
            }
        }
    }
}

static LOGGER: OnceLock<AuditLogger> = OnceLock::new();

/// Initialize the legal audit logger.
pub fn init(config: &LegalAuditConfig) {
    if !config.enabled {
        return;
    }

    let _ = LOGGER.set(AuditLogger::new(config.path.clone(), config.hash_chain));
}

/// Log a legal audit event.
pub fn record(event_type: &str, details: serde_json::Value) {
    if let Some(logger) = LOGGER.get() {
        logger.write(event_type, details);
    }
}

/// Increment the blocked-action counter.
pub fn inc_blocked_action() {
    if let Some(logger) = LOGGER.get() {
        logger.bump_metric(|m| m.blocked_actions += 1);
    }
}

/// Increment the approval-required counter.
pub fn inc_approval_required() {
    if let Some(logger) = LOGGER.get() {
        logger.bump_metric(|m| m.approval_required += 1);
    }
}

/// Increment the redaction-events counter.
pub fn inc_redaction_event() {
    if let Some(logger) = LOGGER.get() {
        logger.bump_metric(|m| m.redaction_events += 1);
    }
}

/// Returns true if audit logging is active.
pub fn enabled() -> bool {
    LOGGER.get().is_some()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::Value;

    use super::AuditLogger;

    #[test]
    fn hash_chain_links_consecutive_events() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("audit.jsonl");
        let logger = AuditLogger::new(path.clone(), true);

        logger.write("first", serde_json::json!({"n": 1}));
        logger.write("second", serde_json::json!({"n": 2}));

        let raw = fs::read_to_string(path).expect("read audit log");
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: Value = serde_json::from_str(lines[0]).expect("first line json");
        let second: Value = serde_json::from_str(lines[1]).expect("second line json");

        let first_hash = first
            .get("hash")
            .and_then(|v| v.as_str())
            .expect("first hash")
            .to_string();
        assert!(first.get("prev_hash").map(|v| v.is_null()).unwrap_or(true));

        let second_prev = second
            .get("prev_hash")
            .and_then(|v| v.as_str())
            .expect("second prev_hash");
        assert_eq!(second_prev, first_hash);
        assert!(second.get("hash").and_then(|v| v.as_str()).is_some());
    }
}
