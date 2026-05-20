//! Lean host-composed no-exposure guard.
//!
//! This service is the host-runtime wrapper around `ironclaw_safety` leak
//! detection. Upper host code should depend on this service instead of wiring
//! `LeakDetector` directly, so production egress policy has one composition
//! seam to grow from.

use ironclaw_safety::{LeakDetectionError, LeakDetector};
use serde_json::{Map, Value};
use std::fmt;
use thiserror::Error;

const MAX_JSON_DEPTH: usize = 64;
const MAX_JSON_NODES: usize = 10_000;

/// Host boundary being protected by [`NoExposureGuard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ExposureBoundary {
    InboundUserText,
    ModelVisibleToolOutput,
    PublicApi,
    SseEvent,
    DurableEvent,
    LogDiagnostic,
}

impl ExposureBoundary {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InboundUserText => "inbound_user_text",
            Self::ModelVisibleToolOutput => "model_visible_tool_output",
            Self::PublicApi => "public_api",
            Self::SseEvent => "sse_event",
            Self::DurableEvent => "durable_event",
            Self::LogDiagnostic => "log_diagnostic",
        }
    }
}

impl fmt::Display for ExposureBoundary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Stable host no-exposure violation.
///
/// This type must never include raw payload text or leak-detector previews.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("no exposure violation at {boundary}: {code}")]
pub struct NoExposureViolation {
    boundary: ExposureBoundary,
    code: &'static str,
}

impl NoExposureViolation {
    pub const CODE: &'static str = "no_exposure_violation";

    fn new(boundary: ExposureBoundary) -> Self {
        Self {
            boundary,
            code: Self::CODE,
        }
    }

    pub fn boundary(&self) -> ExposureBoundary {
        self.boundary
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

/// Host-owned no-exposure service.
pub struct NoExposureGuard {
    detector: LeakDetector,
}

impl NoExposureGuard {
    pub fn new() -> Self {
        Self {
            detector: LeakDetector::new(),
        }
    }

    #[cfg(test)]
    fn with_detector(detector: LeakDetector) -> Self {
        Self { detector }
    }

    /// Check text crossing a host boundary.
    ///
    /// Redactable matches return cleaned text. Blocked matches return a stable
    /// sanitized violation that does not contain the original payload or masked
    /// detector preview.
    pub fn check_text(
        &self,
        boundary: ExposureBoundary,
        text: &str,
    ) -> Result<String, NoExposureViolation> {
        self.detector
            .scan_and_clean(text)
            .map_err(|_| NoExposureViolation::new(boundary))
    }

    /// Recursively check JSON string values and object keys crossing a host boundary.
    pub fn check_json(
        &self,
        boundary: ExposureBoundary,
        value: Value,
    ) -> Result<Value, NoExposureViolation> {
        let mut nodes_seen = 0;
        self.check_json_at_depth(boundary, value, 0, &mut nodes_seen)
    }

    fn check_json_at_depth(
        &self,
        boundary: ExposureBoundary,
        value: Value,
        depth: usize,
        nodes_seen: &mut usize,
    ) -> Result<Value, NoExposureViolation> {
        if depth > MAX_JSON_DEPTH {
            return Err(NoExposureViolation::new(boundary));
        }
        *nodes_seen += 1;
        if *nodes_seen > MAX_JSON_NODES {
            return Err(NoExposureViolation::new(boundary));
        }

        match value {
            Value::String(text) => self.check_text(boundary, &text).map(Value::String),
            Value::Array(values) => values
                .into_iter()
                .map(|value| self.check_json_at_depth(boundary, value, depth + 1, nodes_seen))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            Value::Object(entries) => {
                let mut checked = Map::with_capacity(entries.len());
                for (key, value) in entries {
                    let key = self.check_text(boundary, &key)?;
                    let value = self.check_json_at_depth(boundary, value, depth + 1, nodes_seen)?;
                    if checked.insert(key, value).is_some() {
                        return Err(NoExposureViolation::new(boundary));
                    }
                }
                Ok(Value::Object(checked))
            }
            value => Ok(value),
        }
    }

    /// Check HTTP egress payloads through the host service wrapper.
    pub fn check_http_request(
        &self,
        boundary: ExposureBoundary,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<(), NoExposureViolation> {
        self.check_no_exposure_match(boundary, url)?;
        for (name, value) in headers {
            self.check_no_exposure_match(boundary, name)?;
            self.check_no_exposure_match(boundary, value)?;
        }
        if let Some(body) = body {
            let body = String::from_utf8_lossy(body);
            self.check_no_exposure_match(boundary, &body)?;
        }
        Ok(())
    }

    fn check_no_exposure_match(
        &self,
        boundary: ExposureBoundary,
        text: &str,
    ) -> Result<(), NoExposureViolation> {
        let result = self.detector.scan(text);
        if result.should_block || result.redacted_content.is_some() {
            return Err(NoExposureViolation::new(boundary));
        }
        Ok(())
    }
}

impl fmt::Debug for NoExposureGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NoExposureGuard { detector: LeakDetector }")
    }
}

impl Default for NoExposureGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl From<(ExposureBoundary, LeakDetectionError)> for NoExposureViolation {
    fn from((boundary, _): (ExposureBoundary, LeakDetectionError)) -> Self {
        Self::new(boundary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_safety::{LeakAction, LeakPattern, LeakSeverity};
    use regex::Regex;
    use serde_json::json;

    fn guard_with(pattern: &str, action: LeakAction) -> NoExposureGuard {
        NoExposureGuard::with_detector(LeakDetector::with_patterns(vec![LeakPattern {
            name: "sentinel".to_string(),
            regex: Regex::new(pattern).expect("valid regex"),
            severity: LeakSeverity::Critical,
            action,
        }]))
    }

    #[test]
    fn check_text_redacts_without_blocking() {
        let guard = guard_with("SECRET-[0-9]+", LeakAction::Redact);

        let checked = guard
            .check_text(
                ExposureBoundary::ModelVisibleToolOutput,
                "value=SECRET-12345",
            )
            .expect("redactable payload should pass");

        assert_eq!(checked, "value=[REDACTED]");
    }

    #[test]
    fn check_text_blocks_with_sanitized_error() {
        let guard = guard_with("SECRET-[0-9]+", LeakAction::Block);

        let error = guard
            .check_text(ExposureBoundary::PublicApi, "value=SECRET-12345")
            .expect_err("blocked payload should fail");

        assert_eq!(error.code(), NoExposureViolation::CODE);
        assert_eq!(error.boundary(), ExposureBoundary::PublicApi);
        assert!(!error.to_string().contains("SECRET-12345"));
        assert!(!error.to_string().contains("sentinel"));
    }

    #[test]
    fn check_json_recursively_sanitizes_strings_and_keys() {
        let guard = guard_with("SECRET-[0-9]+", LeakAction::Redact);
        let value = json!({
            "safe": ["SECRET-12345", {"SECRET-67890": "ok"}],
            "number": 1
        });

        let checked = guard
            .check_json(ExposureBoundary::DurableEvent, value)
            .expect("redactable json should pass");

        assert_eq!(
            checked,
            json!({
                "safe": ["[REDACTED]", {"[REDACTED]": "ok"}],
                "number": 1
            })
        );
    }

    #[test]
    fn check_json_blocks_secret_values() {
        let guard = guard_with("SECRET-[0-9]+", LeakAction::Block);
        let value = json!({"nested": {"value": "SECRET-12345"}});

        let error = guard
            .check_json(ExposureBoundary::SseEvent, value)
            .expect_err("blocked json should fail");

        assert_eq!(error.code(), NoExposureViolation::CODE);
        assert_eq!(error.boundary(), ExposureBoundary::SseEvent);
        assert!(!error.to_string().contains("SECRET-12345"));
    }

    #[test]
    fn check_json_fails_when_redacted_keys_collide() {
        let guard = guard_with("SECRET-[0-9]+", LeakAction::Redact);
        let value = json!({
            "SECRET-12345": "first",
            "SECRET-67890": "second"
        });

        let error = guard
            .check_json(ExposureBoundary::DurableEvent, value)
            .expect_err("redacted key collisions should fail closed");

        assert_eq!(error.code(), NoExposureViolation::CODE);
        assert_eq!(error.boundary(), ExposureBoundary::DurableEvent);
    }

    #[test]
    fn check_json_rejects_excessive_depth() {
        let guard = NoExposureGuard::new();
        let mut value = Value::String("ok".to_string());
        for _ in 0..=MAX_JSON_DEPTH {
            value = Value::Array(vec![value]);
        }

        let error = guard
            .check_json(ExposureBoundary::SseEvent, value)
            .expect_err("deep json should fail closed");

        assert_eq!(error.code(), NoExposureViolation::CODE);
        assert_eq!(error.boundary(), ExposureBoundary::SseEvent);
    }

    #[test]
    fn check_http_request_blocks_redactable_matches() {
        let guard = guard_with("Bearer [A-Za-z0-9]{20,}", LeakAction::Redact);

        let error = guard
            .check_http_request(
                ExposureBoundary::PublicApi,
                "https://api.example.test/run",
                &[],
                Some(b"{\"token\":\"Bearer abcdefghij0123456789\"}"),
            )
            .expect_err("redactable HTTP request matches should fail closed");

        assert_eq!(error.code(), NoExposureViolation::CODE);
        assert_eq!(error.boundary(), ExposureBoundary::PublicApi);
    }

    #[test]
    fn check_http_request_scans_header_names() {
        let guard = guard_with("SECRET-[0-9]+", LeakAction::Block);

        let error = guard
            .check_http_request(
                ExposureBoundary::PublicApi,
                "https://api.example.test/run",
                &[("SECRET-12345".to_string(), "value".to_string())],
                None,
            )
            .expect_err("header names should be scanned");

        assert_eq!(error.code(), NoExposureViolation::CODE);
        assert_eq!(error.boundary(), ExposureBoundary::PublicApi);
    }
}
