//! Privacy: the deterministic redactor, the external privacy-filter adapter
//! and its canary, the redaction report, and server-side rescrub.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::redaction::redact_sensitive_json;

use super::*;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RedactionReport {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counts: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pii_labels_present: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub blocked_secret_detected: bool,
}

impl RedactionReport {
    pub(crate) fn increment(&mut self, label: impl Into<String>) {
        *self.counts.entry(label.into()).or_insert(0) += 1;
    }

    pub(crate) fn add_pii_label(&mut self, label: impl Into<String>) {
        let label = label.into();
        if !self.pii_labels_present.contains(&label) {
            self.pii_labels_present.push(label);
        }
    }

    fn add_warning(&mut self, warning: impl Into<String>) {
        let warning = warning.into();
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    fn merge(&mut self, other: RedactionReport) {
        for (key, value) in other.counts {
            *self.counts.entry(key).or_insert(0) += value;
        }
        for label in other.pii_labels_present {
            if !self.pii_labels_present.contains(&label) {
                self.pii_labels_present.push(label);
            }
        }
        for warning in other.warnings {
            self.add_warning(warning);
        }
        self.blocked_secret_detected |= other.blocked_secret_detected;
    }
}

pub fn safe_privacy_filter_redaction_from_output(
    output: &Value,
) -> Result<SafePrivacyFilterRedaction, TraceContributionError> {
    let redacted_text = output
        .get("redacted_text")
        .and_then(Value::as_str)
        .ok_or_else(|| TraceContributionError::RedactionFailed {
            reason: "privacy filter output is missing redacted_text".to_string(),
        })?
        .to_string();
    let detected_spans = output
        .get("detected_spans")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut by_label = BTreeMap::new();
    let mut report = RedactionReport::default();
    for span in &detected_spans {
        let raw_label = span
            .get("label")
            .or_else(|| span.get("type"))
            .or_else(|| span.get("entity_type"))
            .and_then(Value::as_str);
        let label = safe_privacy_filter_label(raw_label, &mut report);
        *by_label.entry(label.clone()).or_insert(0) += 1;
        report.increment(format!("privacy_filter:{label}"));
        if label.eq_ignore_ascii_case("secret") {
            report.blocked_secret_detected = true;
        }
        if !report.pii_labels_present.contains(&label) {
            report.pii_labels_present.push(label);
        }
    }

    let schema_version = output
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(1);
    let decoded_mismatch = output
        .get("decoded_mismatch")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(SafePrivacyFilterRedaction {
        redacted_text,
        summary: SafePrivacyFilterSummary {
            schema_version,
            output_mode: "redacted_text_only".to_string(),
            span_count: detected_spans.len() as u32,
            by_label,
            decoded_mismatch,
        },
        report,
    })
}

pub(crate) fn safe_privacy_filter_label(
    raw_label: Option<&str>,
    report: &mut RedactionReport,
) -> String {
    let Some(raw_label) = raw_label else {
        return "unknown".to_string();
    };
    let normalized = raw_label
        .trim()
        .chars()
        .map(|character| {
            if character == '-' {
                '_'
            } else {
                character.to_ascii_lowercase()
            }
        })
        .collect::<String>();
    let allowed = matches!(
        normalized.as_str(),
        "account_number"
            | "credit_card"
            | "ip_address"
            | "private_address"
            | "private_date"
            | "private_email"
            | "private_location"
            | "private_name"
            | "private_person"
            | "private_phone"
            | "private_url"
            | "secret"
            | "secret_like"
            | "ssn"
    );
    if allowed {
        return normalized;
    }

    report.add_warning("Privacy Filter sidecar emitted unsupported span label; mapped to unknown.");
    "unknown".to_string()
}

pub fn synthetic_privacy_filter_canary_text() -> String {
    synthetic_privacy_filter_canary_values().join(" ")
}

pub fn synthetic_privacy_filter_canary_values() -> Vec<String> {
    vec![
        "trace-canary.person@example.invalid".to_string(),
        "tc_canary_secret_0123456789abcdef".to_string(),
        "/tmp/trace_canary_private/path.txt".to_string(), // safety: canary text intentionally includes a synthetic private path.
    ]
}

pub async fn run_configured_privacy_filter_canary_from_env()
-> Result<Option<PrivacyFilterCanaryReport>, TraceContributionError> {
    let Some(adapter) = privacy_filter_adapter_from_env() else {
        return Ok(None);
    };
    run_privacy_filter_canary(adapter.as_ref()).await.map(Some)
}

pub async fn run_privacy_filter_canary(
    adapter: &dyn PrivacyFilterAdapter,
) -> Result<PrivacyFilterCanaryReport, TraceContributionError> {
    let raw_values = synthetic_privacy_filter_canary_values();
    let canary_text = raw_values.join(" ");
    let canary_hash = canonical_hash(&canary_text);
    let redaction = adapter.redact_text(&canary_text).await?;

    let Some(redaction) = redaction else {
        return Ok(PrivacyFilterCanaryReport {
            canary_version: PRIVACY_FILTER_CANARY_VERSION.to_string(),
            healthy: false,
            canary_hash,
            redacted_output_hash: None,
            summary: None,
            failures: vec!["privacy filter returned no redaction for synthetic canary".to_string()],
        });
    };

    let mut failures = Vec::new();
    let summary_json = serde_json::to_string(&redaction.summary).unwrap_or_default();
    let report_json = serde_json::to_string(&redaction.report).unwrap_or_default();
    for raw_value in &raw_values {
        if redaction.redacted_text.contains(raw_value) {
            failures.push(format!(
                "privacy filter redacted_text retained canary value hash {}",
                canonical_hash(raw_value)
            ));
        }
        if summary_json.contains(raw_value) || report_json.contains(raw_value) {
            failures.push(format!(
                "privacy filter safe summary retained canary value hash {}",
                canonical_hash(raw_value)
            ));
        }
    }

    Ok(PrivacyFilterCanaryReport {
        canary_version: PRIVACY_FILTER_CANARY_VERSION.to_string(),
        healthy: failures.is_empty(),
        canary_hash,
        redacted_output_hash: Some(canonical_hash(&redaction.redacted_text)),
        summary: Some(redaction.summary),
        failures,
    })
}

pub(crate) fn merge_privacy_filter_summary(
    target: &mut Option<SafePrivacyFilterSummary>,
    next: &SafePrivacyFilterSummary,
) {
    let target = target.get_or_insert_with(|| SafePrivacyFilterSummary {
        schema_version: next.schema_version,
        output_mode: "redacted_text_only".to_string(),
        span_count: 0,
        by_label: BTreeMap::new(),
        decoded_mismatch: false,
    });
    target.schema_version = target.schema_version.max(next.schema_version);
    target.span_count = target.span_count.saturating_add(next.span_count);
    target.decoded_mismatch |= next.decoded_mismatch;
    for (label, count) in &next.by_label {
        *target.by_label.entry(label.clone()).or_insert(0) += count;
    }
}

pub(crate) fn redaction_pipeline_version(privacy_filter_used: bool) -> String {
    if privacy_filter_used {
        format!(
            "{DETERMINISTIC_REDACTION_PIPELINE_VERSION}+{PRIVACY_FILTER_SIDECAR_PIPELINE_SUFFIX}"
        )
    } else {
        DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string()
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum TraceContributionError {
    #[error("trace contribution redaction failed: {reason}")]
    RedactionFailed { reason: String },
}

#[async_trait]
pub trait PrivacyFilterAdapter: Send + Sync {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError>;
}

pub struct NoopPrivacyFilterAdapter;

#[async_trait]
impl PrivacyFilterAdapter for NoopPrivacyFilterAdapter {
    async fn redact_text(
        &self,
        _text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        Ok(None)
    }
}

#[derive(Debug, Clone)]
pub struct CommandPrivacyFilterAdapter {
    command: PathBuf,
    args: Vec<String>,
    timeout: Duration,
    max_input_bytes: usize,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
}

impl CommandPrivacyFilterAdapter {
    pub fn new(command: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            timeout: Duration::from_secs(10),
            max_input_bytes: PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES,
            max_stdout_bytes: PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES,
            max_stderr_bytes: PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDERR_BYTES,
        }
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_input_limit(mut self, max_input_bytes: usize) -> Self {
        self.max_input_bytes = max_input_bytes;
        self
    }

    pub fn with_output_limits(mut self, max_stdout_bytes: usize, max_stderr_bytes: usize) -> Self {
        self.max_stdout_bytes = max_stdout_bytes;
        self.max_stderr_bytes = max_stderr_bytes;
        self
    }
}

#[async_trait]
impl PrivacyFilterAdapter for CommandPrivacyFilterAdapter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        if text.len() > self.max_input_bytes {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "privacy filter sidecar input exceeded limit: input_len={} max_input_bytes={}",
                    text.len(),
                    self.max_input_bytes
                ),
            });
        }

        let mut command = tokio::process::Command::new(&self.command);
        command.env_clear();
        for name in ["PATH", "LANG", "LC_ALL"] {
            if let Ok(value) = std::env::var(name) {
                command.env(name, value);
            }
        }
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child =
            command
                .spawn()
                .map_err(|error| TraceContributionError::RedactionFailed {
                    reason: format!(
                        "failed to spawn privacy filter sidecar {}: {}",
                        self.command.display(),
                        error
                    ),
                })?;

        let mut stdin =
            child
                .stdin
                .take()
                .ok_or_else(|| TraceContributionError::RedactionFailed {
                    reason: "privacy filter sidecar stdin was not available".to_string(),
                })?;
        let request = PrivacyFilterSidecarRequest::new(text);
        let request_body = serde_json::to_vec(&request).map_err(|error| {
            TraceContributionError::RedactionFailed {
                reason: format!("failed to serialize privacy filter request: {error}"),
            }
        })?;
        stdin.write_all(&request_body).await.map_err(|error| {
            TraceContributionError::RedactionFailed {
                reason: format!("failed to write privacy filter request: {error}"),
            }
        })?;
        drop(stdin);

        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| TraceContributionError::RedactionFailed {
                reason: format!(
                    "privacy filter sidecar timed out after {}ms",
                    self.timeout.as_millis()
                ),
            })?
            .map_err(|error| TraceContributionError::RedactionFailed {
                reason: format!("privacy filter sidecar failed: {error}"),
            })?;

        if output.stdout.len() > self.max_stdout_bytes {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "stdout exceeded privacy filter sidecar limit: stdout_len={} max_stdout_bytes={}",
                    output.stdout.len(),
                    self.max_stdout_bytes
                ),
            });
        }
        if output.stderr.len() > self.max_stderr_bytes {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "stderr exceeded privacy filter sidecar limit: stderr_len={} stderr_hash={} max_stderr_bytes={}",
                    output.stderr.len(),
                    privacy_filter_bytes_hash(&output.stderr),
                    self.max_stderr_bytes
                ),
            });
        }

        if !output.status.success() {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "privacy filter sidecar exited with {}; stderr_len={} stderr_hash={}",
                    output.status,
                    output.stderr.len(),
                    privacy_filter_bytes_hash(&output.stderr)
                ),
            });
        }

        let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
            TraceContributionError::RedactionFailed {
                reason: format!("failed to parse privacy filter output: {error}"),
            }
        })?;
        safe_privacy_filter_redaction_from_output(&value).map(Some)
    }
}

pub(crate) fn privacy_filter_bytes_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex::encode(digest))
}

pub fn privacy_filter_adapter_from_env() -> Option<Arc<dyn PrivacyFilterAdapter>> {
    let command = std::env::var("IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND").ok()?;
    let command = command.trim();
    if command.is_empty() {
        return None;
    }

    let args = std::env::var("IRONCLAW_TRACE_PRIVACY_FILTER_ARGS")
        .ok()
        .map(|raw| {
            raw.split_whitespace()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut adapter = CommandPrivacyFilterAdapter::new(command).with_args(args);
    if let Some(timeout_ms) = parse_usize_env("IRONCLAW_TRACE_PRIVACY_FILTER_TIMEOUT_MS") {
        adapter = adapter.with_timeout(Duration::from_millis(timeout_ms as u64));
    }
    if let Some(max_input_bytes) = parse_usize_env("IRONCLAW_TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES")
    {
        adapter = adapter.with_input_limit(max_input_bytes);
    }
    let max_stdout = parse_usize_env("IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES");
    let max_stderr = parse_usize_env("IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES");
    if max_stdout.is_some() || max_stderr.is_some() {
        adapter = adapter.with_output_limits(
            max_stdout.unwrap_or(PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES),
            max_stderr.unwrap_or(PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDERR_BYTES),
        );
    }
    Some(Arc::new(adapter))
}

pub(crate) fn parse_usize_env(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

pub struct DeterministicTraceRedactor {
    leak_detector: ironclaw_safety::LeakDetector,
    known_path_prefixes: Vec<String>,
    privacy_filter: Option<Arc<dyn PrivacyFilterAdapter>>,
}

impl Default for DeterministicTraceRedactor {
    fn default() -> Self {
        let mut known_path_prefixes = Vec::new();
        if let Some(home) = dirs::home_dir() {
            known_path_prefixes.push(path_to_string(home));
        }
        if let Ok(current_dir) = std::env::current_dir() {
            known_path_prefixes.push(path_to_string(current_dir));
        }
        Self::new(known_path_prefixes)
    }
}

impl DeterministicTraceRedactor {
    pub fn new(known_path_prefixes: Vec<String>) -> Self {
        let mut known_path_prefixes: Vec<String> = known_path_prefixes
            .into_iter()
            .filter(|prefix| !prefix.trim().is_empty())
            .collect();
        known_path_prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.len()));
        known_path_prefixes.dedup();

        Self {
            leak_detector: ironclaw_safety::LeakDetector::new(),
            known_path_prefixes,
            privacy_filter: privacy_filter_adapter_from_env(),
        }
    }

    pub fn with_privacy_filter(mut self, adapter: Arc<dyn PrivacyFilterAdapter>) -> Self {
        self.privacy_filter = Some(adapter);
        self
    }

    async fn apply_privacy_filter_to_text(
        &self,
        text: String,
        report: &mut RedactionReport,
        privacy_filter_summary: &mut Option<SafePrivacyFilterSummary>,
    ) -> Result<String, TraceContributionError> {
        let Some(adapter) = self.privacy_filter.as_ref() else {
            return Ok(text);
        };
        let redaction = match adapter.redact_text(&text).await {
            Ok(Some(redaction)) => redaction,
            Ok(None) => return Ok(text),
            Err(error) => {
                let error_text = error.to_string();
                report.increment("privacy_filter:sidecar_failure");
                report.add_warning(format!(
                    "Privacy Filter sidecar failed; deterministic redaction fallback was used. error_hash={}",
                    canonical_hash(&error_text)
                ));
                return Ok(text);
            }
        };

        merge_privacy_filter_summary(privacy_filter_summary, &redaction.summary);
        report.merge(redaction.report);
        Ok(redaction.redacted_text)
    }

    pub fn with_known_path_prefixes(prefixes: impl IntoIterator<Item = PathBuf>) -> Self {
        Self::new(prefixes.into_iter().map(path_to_string).collect())
    }

    pub fn redact_text(&self, input: &str) -> (String, RedactionReport) {
        let mut state = RedactionState::default();
        self.redact_text_with_state(input, &mut state)
    }

    fn redact_text_with_state(
        &self,
        input: &str,
        state: &mut RedactionState,
    ) -> (String, RedactionReport) {
        let mut report = RedactionReport::default();
        let mut redacted = self.redact_private_emails(input, state, &mut report);
        redacted = self.redact_generic_paths(&redacted, state, &mut report);
        redacted = self.redact_known_paths(&redacted, state, &mut report);

        let scan = self.leak_detector.scan(&redacted);
        if scan.is_clean() {
            return (redacted, report);
        }

        let ranges = scan
            .matches
            .iter()
            .map(|m| {
                report.increment("secret");
                report.increment(format!("secret:{}", m.pattern_name));
                if matches!(
                    m.severity,
                    ironclaw_safety::LeakSeverity::High | ironclaw_safety::LeakSeverity::Critical
                ) {
                    report.blocked_secret_detected = true;
                }
                m.location.clone()
            })
            .collect::<Vec<_>>();

        (apply_redaction_ranges(&redacted, &ranges), report)
    }

    pub(crate) fn redact_json_value(
        &self,
        tool_name: Option<&str>,
        value: &Value,
        state: &mut RedactionState,
    ) -> (Value, RedactionReport) {
        let mut report = RedactionReport::default();
        let tool_redacted = redact_tool_specific_payload(tool_name, value, &mut report);
        let keyed_redaction = redact_sensitive_json(&tool_redacted);
        count_sensitive_field_redactions(&tool_redacted, &keyed_redaction, &mut report);
        let redacted = self.redact_json_strings(keyed_redaction, state, &mut report);
        (redacted, report)
    }

    fn redact_json_strings(
        &self,
        value: Value,
        state: &mut RedactionState,
        report: &mut RedactionReport,
    ) -> Value {
        match value {
            Value::String(s) => {
                let (redacted, child_report) = self.redact_text_with_state(&s, state);
                report.merge(child_report);
                Value::String(redacted)
            }
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .map(|item| self.redact_json_strings(item, state, report))
                    .collect(),
            ),
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(key, value)| (key, self.redact_json_strings(value, state, report)))
                    .collect(),
            ),
            other => other,
        }
    }

    fn redact_private_emails(
        &self,
        input: &str,
        state: &mut RedactionState,
        report: &mut RedactionReport,
    ) -> String {
        apply_placeholder_regex(input, private_email_regex(), "private_email", state, report)
    }

    fn redact_known_paths(
        &self,
        input: &str,
        state: &mut RedactionState,
        report: &mut RedactionReport,
    ) -> String {
        let mut output = input.to_string();
        for prefix in &self.known_path_prefixes {
            let count = output.matches(prefix).count();
            if count == 0 {
                continue;
            }
            let placeholder = state.placeholders.placeholder_for("local_path", prefix);
            output = output.replace(prefix, &placeholder);
            for _ in 0..count {
                report.increment("local_path");
                report.add_pii_label("local_path");
            }
        }
        output
    }

    fn redact_generic_paths(
        &self,
        input: &str,
        state: &mut RedactionState,
        report: &mut RedactionReport,
    ) -> String {
        apply_placeholder_regex(input, local_path_regex(), "local_path", state, report)
    }
}

#[derive(Debug, Default)]
pub(crate) struct RedactionState {
    pub(crate) placeholders: PlaceholderMap,
}

#[derive(Debug, Default)]
pub(crate) struct PlaceholderMap {
    by_label_and_value: BTreeMap<(String, String), String>,
    next_by_label: BTreeMap<String, u32>,
}

impl PlaceholderMap {
    pub(crate) fn placeholder_for(&mut self, label: &str, value: &str) -> String {
        let key = (label.to_string(), value.to_string());
        if let Some(existing) = self.by_label_and_value.get(&key) {
            return existing.clone();
        }

        let next = self.next_by_label.entry(label.to_string()).or_insert(0);
        *next += 1;
        let token = format!("<PRIVATE_{}_{}>", placeholder_label_fragment(label), *next);
        self.by_label_and_value.insert(key, token.clone());
        token
    }
}

impl DeterministicTraceRedactor {
    pub async fn redact_trace(
        &self,
        trace: RawTraceContribution,
    ) -> Result<TraceContributionEnvelope, TraceContributionError> {
        let mut report = RedactionReport::default();
        let mut state = RedactionState::default();
        let mut privacy_filter_summary = None;
        let mut events = Vec::with_capacity(trace.events.len());
        let trace_card_scopes = trace.consent.scopes.clone();
        let trace_card_channel = trace.ironclaw.channel;
        let trace_card_revocation_handle = trace.contributor.revocation_handle;

        for raw_event in trace.events {
            let redacted_content = match raw_event.content {
                Some(content) => {
                    let (mut redacted, child_report) =
                        self.redact_text_with_state(&content, &mut state);
                    report.merge(child_report);
                    redacted = self
                        .apply_privacy_filter_to_text(
                            redacted,
                            &mut report,
                            &mut privacy_filter_summary,
                        )
                        .await?;
                    Some(redacted)
                }
                None => None,
            };

            let (structured_payload, payload_report) = self.redact_json_value(
                raw_event.tool_name.as_deref(),
                &raw_event.structured_payload,
                &mut state,
            );
            report.merge(payload_report);

            let tool_call_id = raw_event
                .structured_payload
                .get("tool_call_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let tool_category = raw_event.tool_name.as_deref().map(tool_category_for);
            let side_effect = side_effect_for(raw_event.event_type, raw_event.tool_name.as_deref());

            events.push(TraceContributionEvent {
                event_id: raw_event.event_id,
                parent_event_id: None,
                event_type: raw_event.event_type,
                timestamp: raw_event.timestamp,
                redacted_content,
                structured_payload,
                tool_name: raw_event.tool_name,
                tool_category,
                tool_call_id,
                latency_ms: raw_event.latency_ms,
                token_counts: raw_event.token_counts,
                cost_usd: raw_event.cost_usd,
                success: None,
                failure_modes: Vec::new(),
                side_effect,
            });
        }

        let mut outcome = trace.outcome;
        if let Some(correction) = outcome.human_correction.take() {
            let (mut redacted, child_report) = self.redact_text_with_state(&correction, &mut state);
            report.merge(child_report);
            redacted = self
                .apply_privacy_filter_to_text(redacted, &mut report, &mut privacy_filter_summary)
                .await?;
            outcome.human_correction = Some(redacted);
        }

        let residual_pii_risk = residual_risk(&trace.consent, &report);
        let redaction_hash = redaction_hash(&events, &report.counts);
        let mut warnings = privacy_warnings(residual_pii_risk);
        warnings.extend(report.warnings.clone());
        let privacy = PrivacyMetadata {
            redaction_pipeline_version: redaction_pipeline_version(
                privacy_filter_summary.is_some(),
            ),
            redaction_counts: report.counts,
            privacy_filter_summary,
            pii_labels_present: report.pii_labels_present,
            residual_pii_risk,
            redaction_hash,
            warnings,
        };

        let trace_card = build_trace_card(
            &trace_card_scopes,
            trace_card_channel,
            trace_card_revocation_handle,
            &events,
        );
        let value_card = TraceValueCard::default();
        Ok(TraceContributionEnvelope {
            schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
            trace_id: trace.trace_id,
            submission_id: trace.submission_id,
            created_at: trace.created_at,
            ironclaw: trace.ironclaw,
            consent: trace.consent,
            contributor: trace.contributor,
            privacy,
            events,
            outcome,
            replay: trace.replay,
            embedding_analysis: trace.embedding_analysis,
            value: trace.value,
            trace_card,
            value_card,
            hindsight: None,
            training_dynamics: None,
            process_evaluation: None,
            manual_review_authorized: false,
        })
    }
}

pub fn rescrub_trace_envelope(envelope: &mut TraceContributionEnvelope) {
    let redactor = DeterministicTraceRedactor::default();
    rescrub_trace_envelope_with(&redactor, envelope);
}

pub fn rescrub_trace_envelope_with(
    redactor: &DeterministicTraceRedactor,
    envelope: &mut TraceContributionEnvelope,
) {
    let mut report = RedactionReport::default();
    let mut state = RedactionState::default();

    for event in &mut envelope.events {
        if let Some(content) = event.redacted_content.take() {
            let (redacted, child_report) = redactor.redact_text_with_state(&content, &mut state);
            report.merge(child_report);
            event.redacted_content = Some(redacted);
        }

        if !event.structured_payload.is_null() {
            let (redacted_payload, child_report) = redactor.redact_json_value(
                event.tool_name.as_deref(),
                &event.structured_payload,
                &mut state,
            );
            report.merge(child_report);
            event.structured_payload = redacted_payload;
        }
    }

    if let Some(correction) = envelope.outcome.human_correction.take() {
        let (redacted, child_report) = redactor.redact_text_with_state(&correction, &mut state);
        report.merge(child_report);
        envelope.outcome.human_correction = Some(redacted);
    }

    let blocked_secret_detected = report.blocked_secret_detected;
    for (label, count) in report.counts {
        *envelope.privacy.redaction_counts.entry(label).or_insert(0) += count;
    }
    for label in report.pii_labels_present {
        if !envelope.privacy.pii_labels_present.contains(&label) {
            envelope.privacy.pii_labels_present.push(label);
        }
    }

    let server_pass_risk = residual_risk(
        &envelope.consent,
        &RedactionReport {
            counts: BTreeMap::new(),
            pii_labels_present: Vec::new(),
            warnings: Vec::new(),
            blocked_secret_detected,
        },
    );
    envelope.privacy.residual_pii_risk =
        max_residual_risk(envelope.privacy.residual_pii_risk, server_pass_risk);
    if !envelope
        .privacy
        .redaction_pipeline_version
        .contains(SERVER_RESCRUB_PIPELINE_SUFFIX)
    {
        envelope.privacy.redaction_pipeline_version.push('+');
        envelope
            .privacy
            .redaction_pipeline_version
            .push_str(SERVER_RESCRUB_PIPELINE_SUFFIX);
    }
    envelope.trace_card.redaction_pipeline_version =
        envelope.privacy.redaction_pipeline_version.clone();
    merge_privacy_warnings(
        &mut envelope.privacy.warnings,
        privacy_warnings(envelope.privacy.residual_pii_risk),
    );
    merge_privacy_warnings(
        &mut envelope.privacy.warnings,
        vec!["Server-side trace re-scrub was applied before corpus storage.".to_string()],
    );
    envelope.privacy.redaction_hash =
        redaction_hash(&envelope.events, &envelope.privacy.redaction_counts);
}

pub(crate) fn residual_risk(
    consent: &ConsentMetadata,
    report: &RedactionReport,
) -> ResidualPiiRisk {
    if report.blocked_secret_detected {
        return ResidualPiiRisk::High;
    }

    if consent.message_text_included || consent.tool_payloads_included {
        return ResidualPiiRisk::Medium;
    }

    ResidualPiiRisk::Low
}

pub(crate) fn max_residual_risk(left: ResidualPiiRisk, right: ResidualPiiRisk) -> ResidualPiiRisk {
    use ResidualPiiRisk::{High, Low, Medium};
    match (left, right) {
        (High, _) | (_, High) => High,
        (Medium, _) | (_, Medium) => Medium,
        (Low, Low) => Low,
    }
}

pub(crate) fn merge_privacy_warnings(existing: &mut Vec<String>, new_warnings: Vec<String>) {
    for warning in new_warnings {
        if !existing.contains(&warning) {
            existing.push(warning);
        }
    }
}

pub(crate) fn privacy_warnings(risk: ResidualPiiRisk) -> Vec<String> {
    match risk {
        ResidualPiiRisk::Low => Vec::new(),
        ResidualPiiRisk::Medium => vec![
            "Message text or tool payloads were included after local redaction; server-side re-scrub is still required.".to_string(),
        ],
        ResidualPiiRisk::High => vec![
            "Secret-like content was detected after deterministic scrubbing; keep this trace quarantined until reviewed.".to_string(),
        ],
    }
}
