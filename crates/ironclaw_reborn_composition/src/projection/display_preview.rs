use std::{collections::HashMap, sync::Mutex};

use async_trait::async_trait;
use ironclaw_event_projections::{CapabilityActivityProjection, CapabilityActivityStatus};
use ironclaw_host_api::{CapabilityId, InvocationId};
use ironclaw_product_adapters::{
    CAPABILITY_DISPLAY_PREVIEW_MAX_BYTES, CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
    CapabilityDisplayPreviewView, CapabilityDisplayPreviewViewInput, ProductAdapterError,
};
use ironclaw_threads::ThreadMessageId;
use ironclaw_turns::run_profile::CapabilityInputRef;

use super::capability_activity_status_wire;

pub(crate) const SANITIZE_JSON_MAX_DEPTH: usize = 32;

#[async_trait]
pub(super) trait CapabilityDisplayPreviewSource: Send + Sync {
    async fn preview(
        &self,
        activity: &CapabilityActivityProjection,
    ) -> Result<Option<CapabilityDisplayPreviewView>, ProductAdapterError>;
}

pub(super) struct NoopCapabilityDisplayPreviewSource;

#[async_trait]
impl CapabilityDisplayPreviewSource for NoopCapabilityDisplayPreviewSource {
    async fn preview(
        &self,
        _activity: &CapabilityActivityProjection,
    ) -> Result<Option<CapabilityDisplayPreviewView>, ProductAdapterError> {
        Ok(None)
    }
}

#[derive(Default)]
pub(crate) struct CapabilityDisplayPreviewStore {
    pending: Mutex<CapabilityDisplayPendingInputs>,
    completed: Mutex<CapabilityDisplayCompletedPreviews>,
}

#[derive(Default)]
struct CapabilityDisplayPendingInputs {
    by_ref: HashMap<String, CapabilityDisplayInputPreview>,
    refs_by_run: HashMap<String, Vec<String>>,
}

#[derive(Default)]
struct CapabilityDisplayCompletedPreviews {
    by_invocation: HashMap<String, CapabilityDisplayPreviewRecord>,
    invocations_by_run: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
struct CapabilityDisplayInputPreview {
    title: String,
    subtitle: Option<String>,
    input_summary: Option<String>,
    truncated: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct CapabilityDisplayPreviewRecord {
    pub(crate) timeline_message_id: Option<ThreadMessageId>,
    pub(crate) title: String,
    pub(crate) subtitle: Option<String>,
    pub(crate) input_summary: Option<String>,
    pub(crate) output_summary: Option<String>,
    pub(crate) output_preview: Option<String>,
    pub(crate) output_kind: Option<String>,
    pub(crate) output_bytes: Option<u64>,
    pub(crate) result_ref: Option<String>,
    pub(crate) truncated: bool,
}

pub(crate) struct CapabilityDisplayPreviewResult<'a> {
    pub(crate) run_id: &'a str,
    pub(crate) input_ref: &'a CapabilityInputRef,
    pub(crate) invocation_id: InvocationId,
    pub(crate) capability_id: &'a CapabilityId,
    pub(crate) result_ref: &'a str,
    pub(crate) output: &'a serde_json::Value,
    pub(crate) output_bytes: u64,
}

impl CapabilityDisplayPreviewStore {
    pub(crate) fn record_input(
        &self,
        run_id: &str,
        input_ref: &CapabilityInputRef,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) {
        let input_summary = input_summary(tool_name, arguments);
        let input = CapabilityDisplayInputPreview {
            title: bounded_display_text(tool_name, CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES).text,
            subtitle: safe_path_subtitle(arguments),
            truncated: input_summary
                .as_ref()
                .is_some_and(|summary| summary.truncated),
            input_summary: input_summary.map(|summary| summary.text),
        };
        if let Ok(mut pending) = self.pending.lock() {
            let input_ref = input_ref.as_str().to_string();
            pending.by_ref.insert(input_ref.clone(), input);
            pending
                .refs_by_run
                .entry(run_id.to_string())
                .or_default()
                .push(input_ref);
        }
    }

    pub(crate) fn record_result(&self, result: CapabilityDisplayPreviewResult<'_>) {
        let input = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.by_ref.remove(result.input_ref.as_str()));
        let title = input
            .as_ref()
            .map(|input| input.title.clone())
            .unwrap_or_else(|| safe_capability_title(result.capability_id.as_str()).to_string());
        let output = output_preview(result.output);
        let record = CapabilityDisplayPreviewRecord {
            timeline_message_id: None,
            title,
            subtitle: input.as_ref().and_then(|input| input.subtitle.clone()),
            input_summary: input.as_ref().and_then(|input| input.input_summary.clone()),
            output_summary: output.summary,
            output_preview: output.preview,
            output_kind: Some(output.kind),
            output_bytes: Some(result.output_bytes),
            result_ref: Some(result.result_ref.to_string()),
            truncated: input.as_ref().is_some_and(|input| input.truncated) || output.truncated,
        };
        if let Ok(mut completed) = self.completed.lock() {
            let invocation_id = result.invocation_id.to_string();
            completed
                .by_invocation
                .insert(invocation_id.clone(), record);
            completed
                .invocations_by_run
                .entry(result.run_id.to_string())
                .or_default()
                .push(invocation_id);
        }
    }

    pub(crate) fn prune_run(&self, run_id: &str) {
        if let Ok(mut pending) = self.pending.lock()
            && let Some(input_refs) = pending.refs_by_run.remove(run_id)
        {
            for input_ref in input_refs {
                pending.by_ref.remove(&input_ref);
            }
        }
        if let Ok(mut completed) = self.completed.lock()
            && let Some(invocation_ids) = completed.invocations_by_run.remove(run_id)
        {
            for invocation_id in invocation_ids {
                completed.by_invocation.remove(&invocation_id);
            }
        }
    }

    pub(crate) fn record_for_invocation(
        &self,
        invocation_id: InvocationId,
    ) -> Option<CapabilityDisplayPreviewRecord> {
        self.completed.lock().ok().and_then(|completed| {
            completed
                .by_invocation
                .get(&invocation_id.to_string())
                .cloned()
        })
    }

    pub(crate) fn attach_timeline_message_id(
        &self,
        invocation_id: InvocationId,
        timeline_message_id: ThreadMessageId,
    ) {
        if let Ok(mut completed) = self.completed.lock()
            && let Some(record) = completed.by_invocation.get_mut(&invocation_id.to_string())
        {
            record.timeline_message_id = Some(timeline_message_id);
        }
    }
}

#[async_trait]
impl CapabilityDisplayPreviewSource for CapabilityDisplayPreviewStore {
    async fn preview(
        &self,
        activity: &CapabilityActivityProjection,
    ) -> Result<Option<CapabilityDisplayPreviewView>, ProductAdapterError> {
        capability_display_preview_from_store(self, activity)
    }
}

fn capability_display_preview_from_store(
    store: &CapabilityDisplayPreviewStore,
    activity: &CapabilityActivityProjection,
) -> Result<Option<CapabilityDisplayPreviewView>, ProductAdapterError> {
    if !matches!(
        activity.status,
        CapabilityActivityStatus::Completed
            | CapabilityActivityStatus::Failed
            | CapabilityActivityStatus::Killed
    ) {
        return Ok(None);
    }
    let Some(record) = store.record_for_invocation(activity.invocation_id) else {
        return failed_capability_display_preview(activity);
    };
    CapabilityDisplayPreviewView::new(CapabilityDisplayPreviewViewInput {
        timeline_message_id: record
            .timeline_message_id
            .map(|message_id| message_id.to_string()),
        invocation_id: activity.invocation_id,
        thread_id: activity.thread_id.clone(),
        capability_id: activity.capability_id.clone(),
        status: capability_activity_status_wire(activity.status),
        title: record.title,
        subtitle: record.subtitle,
        input_summary: record.input_summary,
        output_summary: record.output_summary,
        output_preview: record.output_preview,
        output_kind: record.output_kind,
        output_bytes: activity.output_bytes.or(record.output_bytes),
        result_ref: record.result_ref,
        truncated: record.truncated,
        updated_at: activity.updated_at,
    })
    .map(Some)
}

fn failed_capability_display_preview(
    activity: &CapabilityActivityProjection,
) -> Result<Option<CapabilityDisplayPreviewView>, ProductAdapterError> {
    if !matches!(
        activity.status,
        CapabilityActivityStatus::Failed | CapabilityActivityStatus::Killed
    ) {
        return Ok(None);
    }
    let summary = activity
        .error_kind
        .as_deref()
        .map(|kind| format!("tool failed: {}", sanitize_text(kind)))
        .unwrap_or_else(|| "tool failed".to_string());
    CapabilityDisplayPreviewView::new(CapabilityDisplayPreviewViewInput {
        timeline_message_id: None,
        invocation_id: activity.invocation_id,
        thread_id: activity.thread_id.clone(),
        capability_id: activity.capability_id.clone(),
        status: capability_activity_status_wire(activity.status),
        title: bounded_display_text(
            safe_capability_title(activity.capability_id.as_str()),
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        )
        .text,
        subtitle: None,
        input_summary: None,
        output_summary: Some(summary.clone()),
        output_preview: Some(summary),
        output_kind: Some("text".to_string()),
        output_bytes: activity.output_bytes,
        result_ref: None,
        truncated: false,
        updated_at: activity.updated_at,
    })
    .map(Some)
}

#[derive(Debug, Clone)]
struct OutputPreview {
    summary: Option<String>,
    preview: Option<String>,
    kind: String,
    truncated: bool,
}

fn output_preview(value: &serde_json::Value) -> OutputPreview {
    let (kind, text, json_truncated) = if let Some(text) = value.as_str() {
        ("text", text.to_string(), false)
    } else if let Some(text) = value
        .get("content")
        .or_else(|| value.get("text"))
        .or_else(|| value.get("stdout"))
        .and_then(serde_json::Value::as_str)
    {
        ("text", text.to_string(), false)
    } else {
        let safe_value = sanitize_json_value_with_truncation(value);
        (
            "json",
            serde_json::to_string_pretty(&safe_value.value).unwrap_or_else(|_| "{}".to_string()),
            safe_value.truncated,
        )
    };
    let preview = bounded_preview_text(&text);
    let summary = bounded_display_text(
        match kind {
            "text" => "text output",
            _ => "json output",
        },
        CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
    );
    OutputPreview {
        summary: non_empty(summary.text),
        preview: non_empty(preview.text),
        kind: kind.to_string(),
        truncated: summary.truncated || preview.truncated || json_truncated,
    }
}

#[derive(Debug, Clone)]
struct DisplayText {
    text: String,
    truncated: bool,
}

fn input_summary(capability_id: &str, value: &serde_json::Value) -> Option<DisplayText> {
    if (capability_id == "read_file"
        || capability_id == "builtin.read_file"
        || capability_id.ends_with(".read_file"))
        && let Some(path) = safe_path_subtitle(value)
    {
        let mut summary = format!("path: {path}");
        if let Some(max_bytes) = value.get("max_bytes").and_then(serde_json::Value::as_u64) {
            summary.push_str(&format!("\nmax_bytes: {max_bytes}"));
        }
        return Some(bounded_display_text(
            &summary,
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        ));
    }
    let safe_value = sanitize_json_value_with_truncation(value);
    serde_json::to_string_pretty(&safe_value.value)
        .ok()
        .map(|text| {
            let mut summary = bounded_display_text(&text, CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES);
            summary.truncated |= safe_value.truncated;
            summary
        })
}

fn safe_capability_title(capability_id: &str) -> &str {
    capability_id
        .rsplit_once('.')
        .map(|(_, suffix)| suffix)
        .unwrap_or(capability_id)
}

fn safe_path_subtitle(value: &serde_json::Value) -> Option<String> {
    let path = value
        .get("path")
        .or_else(|| value.get("file_path"))
        .or_else(|| value.get("target"))?
        .as_str()?;
    safe_display_path(path)
}

fn safe_display_path(path: &str) -> Option<String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.starts_with('~')
        || path.contains("..")
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return None;
    }
    Some(bounded_display_text(path, CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES).text)
}

#[derive(Debug, Clone)]
struct SanitizedJson {
    value: serde_json::Value,
    truncated: bool,
}

#[cfg(test)]
pub(crate) fn sanitize_json_value(value: &serde_json::Value) -> serde_json::Value {
    sanitize_json_value_with_truncation(value).value
}

fn sanitize_json_value_with_truncation(value: &serde_json::Value) -> SanitizedJson {
    sanitize_json_value_at_depth(value, SANITIZE_JSON_MAX_DEPTH)
}

fn sanitize_json_value_at_depth(
    value: &serde_json::Value,
    remaining_depth: usize,
) -> SanitizedJson {
    if remaining_depth == 0 {
        return SanitizedJson {
            value: serde_json::Value::String("[truncated]".to_string()),
            truncated: true,
        };
    }
    match value {
        serde_json::Value::Object(map) => {
            let mut truncated = false;
            let value = serde_json::Value::Object(
                map.iter()
                    .map(|(key, value)| {
                        let sanitized = if is_sensitive_key(key) {
                            serde_json::Value::String("[redacted]".to_string())
                        } else {
                            let sanitized =
                                sanitize_json_value_at_depth(value, remaining_depth - 1);
                            truncated |= sanitized.truncated;
                            sanitized.value
                        };
                        (key.clone(), sanitized)
                    })
                    .collect(),
            );
            SanitizedJson { value, truncated }
        }
        serde_json::Value::Array(values) => {
            let mut truncated = false;
            let value = serde_json::Value::Array(
                values
                    .iter()
                    .map(|value| {
                        let sanitized = sanitize_json_value_at_depth(value, remaining_depth - 1);
                        truncated |= sanitized.truncated;
                        sanitized.value
                    })
                    .collect(),
            );
            SanitizedJson { value, truncated }
        }
        serde_json::Value::String(value) => SanitizedJson {
            value: serde_json::Value::String(sanitize_text(value)),
            truncated: false,
        },
        other => SanitizedJson {
            value: other.clone(),
            truncated: false,
        },
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let compact_key = key.replace(['_', '-'], "");
    // Display previews bias toward over-redaction; benign counters like max_tokens can be hidden.
    key.contains("secret")
        || key.contains("password")
        || key.contains("token")
        || key.contains("credential")
        || key.contains("api_key")
        || compact_key.contains("apikey")
        || key == "key"
}

fn bounded_display_text(text: &str, max_bytes: usize) -> DisplayText {
    let sanitized = sanitize_text(text);
    truncate_bytes(&sanitized, max_bytes)
}

fn bounded_preview_text(text: &str) -> DisplayText {
    let sanitized = sanitize_text(text);
    truncate_bytes(&sanitized, CAPABILITY_DISPLAY_PREVIEW_MAX_BYTES)
}

fn non_empty(text: String) -> Option<String> {
    if text.is_empty() { None } else { Some(text) }
}

fn truncate_bytes(text: &str, max_bytes: usize) -> DisplayText {
    if text.len() <= max_bytes {
        return DisplayText {
            text: text.to_string(),
            truncated: false,
        };
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    DisplayText {
        text: text[..end].to_string(), // safety: end is adjusted to a UTF-8 char boundary above.
        truncated: true,
    }
}

pub(crate) fn sanitize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut redact_next_value = false;
    for token in text.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim_end();
        if trimmed.is_empty() {
            push_safe_text(&mut out, token);
            continue;
        }
        let suffix = &token[trimmed.len()..];
        let redact_current =
            redact_next_value || is_secret_like(trimmed) || is_unsafe_path_like(trimmed);
        if redact_current {
            out.push_str("[redacted]");
            push_safe_text(&mut out, suffix);
        } else {
            push_safe_text(&mut out, token);
        }
        redact_next_value = credential_key_expects_value(trimmed) && !suffix.is_empty();
    }
    out
}

fn push_safe_text(out: &mut String, text: &str) {
    out.extend(
        text.chars().filter(|character| {
            *character == '\n' || *character == '\t' || !character.is_control()
        }),
    );
}

fn is_secret_like(token: &str) -> bool {
    let trimmed = token.trim_matches(token_boundary_punctuation);
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("sk-")
        || lower.starts_with("ghp_")
        || lower.starts_with("gho_")
        || lower.starts_with("ghu_")
        || lower.starts_with("ghs_")
        || lower.starts_with("xoxb-")
        || lower.starts_with("xoxa-")
        || lower.starts_with("xoxp-")
        || looks_like_aws_access_key(trimmed)
        || looks_like_jwt(trimmed)
        || lower.contains("api_key=")
        || lower.contains("api_key:")
        || lower.contains("apikey=")
        || lower.contains("apikey:")
        || lower.contains("access_token=")
        || lower.contains("access_token:")
        || lower.contains("secret=")
        || lower.contains("secret:")
        || lower.contains("password=")
        || lower.contains("password:")
        || lower.contains("token=")
        || lower.contains("token:")
}

fn is_unsafe_path_like(token: &str) -> bool {
    let token = token.trim_matches(token_boundary_punctuation);
    token.to_ascii_lowercase().starts_with("file:/")
        || token_contains_absolute_posix_path(token)
        || token.starts_with("\\\\")
        || token.contains("\\\\")
        || token.get(1..3) == Some(":\\")
}

fn credential_key_expects_value(token: &str) -> bool {
    let lower = token
        .trim_matches(non_credential_boundary_punctuation)
        .to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "api_key:"
            | "api_key="
            | "apikey:"
            | "apikey="
            | "access_token:"
            | "access_token="
            | "secret:"
            | "secret="
            | "password:"
            | "password="
            | "token:"
            | "token="
    )
}

fn non_credential_boundary_punctuation(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
    )
}

fn looks_like_aws_access_key(token: &str) -> bool {
    (token.starts_with("AKIA") || token.starts_with("ASIA"))
        && token.len() >= 16
        && token
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn looks_like_jwt(token: &str) -> bool {
    token.starts_with("eyJ")
        && token.matches('.').count() >= 2
        && token.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn token_contains_absolute_posix_path(token: &str) -> bool {
    let mut previous = None;
    let mut characters = token.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '/'
            && previous.is_none_or(token_boundary_punctuation)
            && !matches!(previous, Some('/'))
            && !matches!(characters.peek(), Some('/'))
        {
            return true;
        }
        previous = Some(character);
    }
    false
}

fn token_boundary_punctuation(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '`' | ',' | ';' | ':' | '=' | '(' | ')' | '[' | ']' | '{' | '}'
    )
}
