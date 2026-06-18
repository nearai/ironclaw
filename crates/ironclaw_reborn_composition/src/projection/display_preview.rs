use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};

use async_trait::async_trait;
use ironclaw_event_projections::{CapabilityActivityProjection, CapabilityActivityStatus};
use ironclaw_host_api::{
    CapabilityDisplayOutputPreview, CapabilityDisplayText, CapabilityId, InvocationId,
    truncate_capability_display_text,
};
use ironclaw_product_adapters::{
    CAPABILITY_DISPLAY_PREVIEW_MAX_BYTES, CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
    CapabilityDisplayPreviewView, CapabilityDisplayPreviewViewInput, ProductAdapterError,
};
use ironclaw_safety::{
    sanitize_display_text as safety_sanitize_text, sanitize_url_for_display,
    shell_command_display_text,
};
use ironclaw_threads::ThreadMessageId;
use ironclaw_turns::{TurnRunId, run_profile::CapabilityInputRef};

use super::capability_activity_status_wire;

pub(crate) const SANITIZE_JSON_MAX_DEPTH: usize = 32;
const COMPLETED_PREVIEW_PENDING_TIMEOUT_SECONDS: i64 = 10;

#[async_trait]
pub(super) trait CapabilityDisplayPreviewSource: Send + Sync {
    async fn preview_resolution(
        &self,
        activity: &CapabilityActivityProjection,
    ) -> Result<CapabilityDisplayPreviewResolution, ProductAdapterError>;

    /// Input a still-running invocation should display inline, or `None` if the
    /// invocation is not in-flight (or has no recorded input). Default `None`.
    fn running_input(&self, _invocation_id: InvocationId) -> Option<CapabilityRunningInput> {
        None
    }

    #[cfg(test)]
    async fn preview(
        &self,
        activity: &CapabilityActivityProjection,
    ) -> Result<Option<CapabilityDisplayPreviewView>, ProductAdapterError> {
        Ok(match self.preview_resolution(activity).await? {
            CapabilityDisplayPreviewResolution::Ready(preview) => Some(*preview),
            CapabilityDisplayPreviewResolution::Pending
            | CapabilityDisplayPreviewResolution::NotApplicable => None,
        })
    }
}

pub(super) struct NoopCapabilityDisplayPreviewSource;

pub(super) enum CapabilityDisplayPreviewResolution {
    Ready(Box<CapabilityDisplayPreviewView>),
    Pending,
    NotApplicable,
}

#[async_trait]
impl CapabilityDisplayPreviewSource for NoopCapabilityDisplayPreviewSource {
    async fn preview_resolution(
        &self,
        _activity: &CapabilityActivityProjection,
    ) -> Result<CapabilityDisplayPreviewResolution, ProductAdapterError> {
        Ok(CapabilityDisplayPreviewResolution::NotApplicable)
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
    /// `invocation_id -> input_ref`, established when the invocation starts
    /// executing (the activity frame only knows the invocation id, but the
    /// input was recorded under its ref at registration). Lets the
    /// still-running activity frame surface the input before the result lands.
    input_ref_by_invocation: HashMap<String, String>,
}

/// The input a still-running invocation shows in its activity row.
#[derive(Debug, Clone, Default)]
pub(super) struct CapabilityRunningInput {
    pub(super) subtitle: Option<String>,
    pub(super) input_summary: Option<String>,
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
    fn lock_pending_inputs(&self) -> MutexGuard<'_, CapabilityDisplayPendingInputs> {
        self.pending.lock().unwrap_or_else(|poisoned| {
            tracing::debug!(
                "capability display preview pending input store was poisoned; recovering"
            );
            poisoned.into_inner()
        })
    }

    fn lock_completed_previews(&self) -> MutexGuard<'_, CapabilityDisplayCompletedPreviews> {
        self.completed.lock().unwrap_or_else(|poisoned| {
            tracing::debug!(
                "capability display preview completed preview store was poisoned; recovering"
            );
            poisoned.into_inner()
        })
    }

    fn has_pending_input_for_activity(&self, activity: &CapabilityActivityProjection) -> bool {
        let Some(run_id) = activity.run_id else {
            return false;
        };
        let pending = self.lock_pending_inputs();
        pending
            .refs_by_run
            .get(&run_id.to_string())
            .is_some_and(|input_refs| {
                input_refs
                    .iter()
                    .any(|input_ref| pending.by_ref.contains_key(input_ref))
            })
    }

    pub(crate) fn record_input(
        &self,
        run_id: &str,
        input_ref: &CapabilityInputRef,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) {
        let mut pending = self.lock_pending_inputs();
        let input_summary = input_summary(tool_name, arguments);
        let input = CapabilityDisplayInputPreview {
            title: bounded_display_text(tool_name, CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES).text,
            subtitle: primary_arg_subtitle(tool_name, arguments),
            truncated: input_summary
                .as_ref()
                .is_some_and(|summary| summary.truncated),
            input_summary: input_summary.map(|summary| summary.text),
        };
        let input_ref = input_ref.as_str().to_string();
        pending.by_ref.insert(input_ref.clone(), input);
        pending
            .refs_by_run
            .entry(run_id.to_string())
            .or_default()
            .push(input_ref);
    }

    /// Link a now-running invocation to the ref its input was recorded under,
    /// so the running activity frame can surface that input before completion.
    pub(crate) fn record_running_invocation(
        &self,
        invocation_id: InvocationId,
        input_ref: &CapabilityInputRef,
    ) {
        let mut pending = self.lock_pending_inputs();
        pending
            .input_ref_by_invocation
            .insert(invocation_id.to_string(), input_ref.as_str().to_string());
    }

    #[cfg(test)]
    pub(crate) fn record_result(&self, result: CapabilityDisplayPreviewResult<'_>) {
        self.record_result_with_preview(result, None);
    }

    pub(crate) fn record_result_with_preview(
        &self,
        result: CapabilityDisplayPreviewResult<'_>,
        display_preview: Option<&CapabilityDisplayOutputPreview>,
    ) {
        let input = {
            let mut pending = self.lock_pending_inputs();
            // The result has landed: drop the running-input link so the
            // activity frame stops surfacing it as in-flight.
            pending
                .input_ref_by_invocation
                .remove(&result.invocation_id.to_string());
            pending.by_ref.remove(result.input_ref.as_str())
        };
        let title = input
            .as_ref()
            .map(|input| input.title.clone())
            .unwrap_or_else(|| safe_capability_title(result.capability_id.as_str()).to_string());
        let output = display_preview
            .map(output_preview_from_display)
            .unwrap_or_else(|| output_preview(result.output));
        let record = CapabilityDisplayPreviewRecord {
            timeline_message_id: None,
            title,
            subtitle: display_preview
                .and_then(|preview| preview.subtitle.as_deref().and_then(safe_preview_subtitle))
                .or_else(|| input.as_ref().and_then(|input| input.subtitle.clone())),
            input_summary: input.as_ref().and_then(|input| input.input_summary.clone()),
            output_summary: output.summary,
            output_preview: output.preview,
            output_kind: Some(output.kind),
            output_bytes: Some(result.output_bytes),
            result_ref: Some(result.result_ref.to_string()),
            truncated: input.as_ref().is_some_and(|input| input.truncated) || output.truncated,
        };
        let mut completed = self.lock_completed_previews();
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

    pub(crate) fn prune_run(&self, run_id: &str) {
        let mut pending = self.lock_pending_inputs();
        if let Some(input_refs) = pending.refs_by_run.remove(run_id) {
            let pruned: std::collections::HashSet<String> = input_refs.into_iter().collect();
            for input_ref in &pruned {
                pending.by_ref.remove(input_ref);
            }
            pending
                .input_ref_by_invocation
                .retain(|_, input_ref| !pruned.contains(input_ref));
        }
        drop(pending);

        let mut completed = self.lock_completed_previews();
        if let Some(invocation_ids) = completed.invocations_by_run.remove(run_id) {
            for invocation_id in invocation_ids {
                completed.by_invocation.remove(&invocation_id);
            }
        }
    }

    pub(crate) fn record_for_invocation(
        &self,
        invocation_id: InvocationId,
    ) -> Option<CapabilityDisplayPreviewRecord> {
        self.lock_completed_previews()
            .by_invocation
            .get(&invocation_id.to_string())
            .cloned()
    }

    pub(crate) fn attach_timeline_message_id(
        &self,
        invocation_id: InvocationId,
        timeline_message_id: ThreadMessageId,
    ) {
        let mut completed = self.lock_completed_previews();
        if let Some(record) = completed.by_invocation.get_mut(&invocation_id.to_string()) {
            record.timeline_message_id = Some(timeline_message_id);
        }
    }
}

#[async_trait]
impl CapabilityDisplayPreviewSource for CapabilityDisplayPreviewStore {
    async fn preview_resolution(
        &self,
        activity: &CapabilityActivityProjection,
    ) -> Result<CapabilityDisplayPreviewResolution, ProductAdapterError> {
        capability_display_preview_resolution_from_store(self, activity)
    }

    fn running_input(&self, invocation_id: InvocationId) -> Option<CapabilityRunningInput> {
        let pending = self.lock_pending_inputs();
        let input_ref = pending
            .input_ref_by_invocation
            .get(&invocation_id.to_string())?;
        let input = pending.by_ref.get(input_ref)?;
        Some(CapabilityRunningInput {
            subtitle: input.subtitle.clone(),
            input_summary: input.input_summary.clone(),
        })
    }
}

fn capability_display_preview_resolution_from_store(
    store: &CapabilityDisplayPreviewStore,
    activity: &CapabilityActivityProjection,
) -> Result<CapabilityDisplayPreviewResolution, ProductAdapterError> {
    if !matches!(
        activity.status,
        CapabilityActivityStatus::Completed
            | CapabilityActivityStatus::Failed
            | CapabilityActivityStatus::Killed
    ) {
        // A still-running invocation has no preview yet, but it WILL produce one
        // when it reaches a terminal status. Resolving it as `Pending` (rather
        // than `NotApplicable`) holds the runtime cursor at this activity's
        // preview slot instead of skipping past it. Skipping was unsafe: the
        // drain would deliver later activities' payloads past this slot, and
        // when the invocation later completed its now-materialized preview sat
        // behind the resume watermark and was never delivered — the dropped
        // dropped tool card. Holding keeps the slot positionally stable so
        // the preview is delivered in order once it lands.
        return Ok(CapabilityDisplayPreviewResolution::Pending);
    }
    let Some(record) = store.record_for_invocation(activity.invocation_id) else {
        return if matches!(
            activity.status,
            CapabilityActivityStatus::Failed | CapabilityActivityStatus::Killed
        ) {
            failed_capability_display_preview(activity)
        } else if store.has_pending_input_for_activity(activity)
            && completed_preview_may_still_arrive(activity)
        {
            Ok(CapabilityDisplayPreviewResolution::Pending)
        } else {
            Ok(CapabilityDisplayPreviewResolution::NotApplicable)
        };
    };
    CapabilityDisplayPreviewView::new(CapabilityDisplayPreviewViewInput {
        timeline_message_id: record
            .timeline_message_id
            .map(|message_id| message_id.to_string()),
        invocation_id: activity.invocation_id,
        turn_run_id: turn_run_id_for_activity(activity),
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
        activity_order: Some(activity.activity_order_cursor().as_u64()),
    })
    .map(Box::new)
    .map(CapabilityDisplayPreviewResolution::Ready)
}

fn completed_preview_may_still_arrive(activity: &CapabilityActivityProjection) -> bool {
    chrono::Utc::now().signed_duration_since(activity.updated_at)
        <= chrono::Duration::seconds(COMPLETED_PREVIEW_PENDING_TIMEOUT_SECONDS)
}

fn failed_capability_display_preview(
    activity: &CapabilityActivityProjection,
) -> Result<CapabilityDisplayPreviewResolution, ProductAdapterError> {
    let summary = activity
        .error_kind
        .as_deref()
        .map(|kind| format!("tool failed: {}", sanitize_text(kind)))
        .unwrap_or_else(|| "tool failed".to_string());
    CapabilityDisplayPreviewView::new(CapabilityDisplayPreviewViewInput {
        timeline_message_id: None,
        invocation_id: activity.invocation_id,
        turn_run_id: turn_run_id_for_activity(activity),
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
        activity_order: Some(activity.activity_order_cursor().as_u64()),
    })
    .map(Box::new)
    .map(CapabilityDisplayPreviewResolution::Ready)
}

fn turn_run_id_for_activity(activity: &CapabilityActivityProjection) -> Option<TurnRunId> {
    activity
        .run_id
        .map(|run_id| TurnRunId::from_uuid(run_id.as_uuid()))
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
        ("text".to_string(), text.to_string(), false)
    } else if let Some(text) = value
        .get("content")
        .or_else(|| value.get("text"))
        .or_else(|| value.get("stdout"))
        .and_then(serde_json::Value::as_str)
    {
        ("text".to_string(), text.to_string(), false)
    } else {
        let safe_value = sanitize_json_value_with_truncation(value);
        (
            "json".to_string(),
            serde_json::to_string_pretty(&safe_value.value).unwrap_or_else(|_| "{}".to_string()),
            safe_value.truncated,
        )
    };
    let preview = bounded_preview_text(&text);
    let summary = bounded_display_text(
        if kind == "text" {
            "text output"
        } else {
            "json output"
        },
        CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
    );
    OutputPreview {
        summary: non_empty(summary.text),
        preview: non_empty(preview.text),
        kind,
        truncated: summary.truncated || preview.truncated || json_truncated,
    }
}

fn output_preview_from_display(value: &CapabilityDisplayOutputPreview) -> OutputPreview {
    let summary = value
        .output_summary
        .as_deref()
        .map(|summary| bounded_display_text(summary, CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES));
    let summary_truncated = summary.as_ref().is_some_and(|summary| summary.truncated);
    let preview = bounded_preview_text(&value.output_preview);
    OutputPreview {
        summary: summary.and_then(|summary| non_empty(summary.text)),
        preview: non_empty(preview.text),
        kind: value.output_kind.clone(),
        truncated: value.truncated || summary_truncated || preview.truncated,
    }
}

fn input_summary(capability_id: &str, value: &serde_json::Value) -> Option<CapabilityDisplayText> {
    if capability_matches(capability_id, "shell")
        && let Some(command) = value.get("command").and_then(serde_json::Value::as_str)
    {
        let command = shell_command_display_text(command);
        let mut summary = bounded_display_text(
            &format!("command: {}", command.text),
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        );
        summary.truncated |= command.truncated;
        return Some(summary);
    }

    if capability_matches(capability_id, "http")
        || capability_matches(capability_id, "http.save")
        || capability_id == "web_fetch"
        || capability_id.ends_with(".web_fetch")
        || capability_id == "web-access.get_content"
    {
        let mut summary = SummaryBuilder::default();
        if let Some(method) = string_arg(value, &["method"]) {
            summary.push(
                "method",
                bounded_summary_value(&method.to_ascii_uppercase()).text,
            );
        }
        if let Some(url) = string_arg(value, &["url"]) {
            let url = safe_url_display(url);
            summary.push_with_truncation("url", url.text, url.truncated);
        }
        if let Some(save_to) = safe_path_arg(value, &["save_to", "output_path", "path"]) {
            summary.push("save_to", save_to);
        }
        if let Some(response_body_limit) = u64_arg(value, &["response_body_limit"]) {
            summary.push("response_body_limit", response_body_limit.to_string());
        }
        if let Some(timeout_ms) = u64_arg(value, &["timeout_ms"]) {
            summary.push("timeout_ms", timeout_ms.to_string());
        }
        if let Some(summary) = summary.finish() {
            return Some(summary);
        }
    }

    if capability_matches(capability_id, "read_file")
        || capability_matches(capability_id, "memory_read")
        || capability_matches(capability_id, "memory_tree")
    {
        let mut summary = SummaryBuilder::default();
        let path = safe_path_arg(value, &["path", "file_path", "target"])
            .or_else(|| capability_matches(capability_id, "memory_tree").then(|| "/".to_string()));
        if let Some(path) = path {
            summary.push("path", path);
        }
        if let Some(offset) = u64_arg(value, &["offset"]) {
            summary.push("offset", offset.to_string());
        }
        if let Some(limit) = u64_arg(value, &["limit", "max_bytes"]) {
            summary.push("limit", limit.to_string());
        }
        if let Some(summary) = summary.finish() {
            return Some(summary);
        }
    }

    if capability_matches(capability_id, "write_file") {
        let mut summary = SummaryBuilder::default();
        if let Some(path) = safe_path_arg(value, &["path", "file_path", "target"]) {
            summary.push("path", path);
        }
        if let Some(content) = string_arg(value, &["content"]) {
            summary.push("content_bytes", content.len().to_string());
        }
        if let Some(summary) = summary.finish() {
            return Some(summary);
        }
    }

    if capability_matches(capability_id, "list_dir") {
        let mut summary = SummaryBuilder::default();
        if let Some(path) = safe_path_arg(value, &["path"]) {
            summary.push("path", path);
        }
        if let Some(recursive) = bool_arg(value, &["recursive"]) {
            summary.push("recursive", recursive.to_string());
        }
        if let Some(max_depth) = u64_arg(value, &["max_depth"]) {
            summary.push("max_depth", max_depth.to_string());
        }
        if let Some(summary) = summary.finish() {
            return Some(summary);
        }
    }

    if capability_matches(capability_id, "glob") {
        let mut summary = SummaryBuilder::default();
        push_text_arg(&mut summary, value, "pattern", &["pattern"]);
        if let Some(path) = safe_path_arg(value, &["path"]) {
            summary.push("path", path);
        }
        if let Some(max_results) = u64_arg(value, &["max_results"]) {
            summary.push("max_results", max_results.to_string());
        }
        if let Some(summary) = summary.finish() {
            return Some(summary);
        }
    }

    if capability_matches(capability_id, "grep") {
        let mut summary = SummaryBuilder::default();
        push_text_arg(&mut summary, value, "pattern", &["pattern"]);
        if let Some(path) = safe_path_arg(value, &["path"]) {
            summary.push("path", path);
        }
        push_text_arg(&mut summary, value, "glob", &["glob"]);
        push_text_arg(&mut summary, value, "output_mode", &["output_mode"]);
        push_number_arg(&mut summary, value, "head_limit", &["head_limit"]);
        push_number_arg(&mut summary, value, "offset", &["offset"]);
        if let Some(summary) = summary.finish() {
            return Some(summary);
        }
    }

    if capability_matches(capability_id, "apply_patch") {
        let mut summary = SummaryBuilder::default();
        if let Some(path) = safe_path_arg(value, &["path", "file_path", "target"]) {
            summary.push("path", path);
        }
        if let Some(old_string) = string_arg(value, &["old_string"]) {
            summary.push("old_bytes", old_string.len().to_string());
        }
        if let Some(new_string) = string_arg(value, &["new_string"]) {
            summary.push("new_bytes", new_string.len().to_string());
        }
        if let Some(replace_all) = bool_arg(value, &["replace_all"]) {
            summary.push("replace_all", replace_all.to_string());
        }
        if let Some(summary) = summary.finish() {
            return Some(summary);
        }
    }

    if capability_matches(capability_id, "memory_search")
        || capability_id == "web_search"
        || capability_id == "llm_context"
        || capability_id.ends_with(".web_search")
        || capability_id.ends_with(".search")
        || capability_id == "web-access.search"
        || capability_id == "nearai.web_search"
    {
        let mut summary = SummaryBuilder::default();
        push_text_arg(
            &mut summary,
            value,
            "query",
            &["query", "q", "text", "pattern"],
        );
        push_number_arg(&mut summary, value, "limit", &["limit", "max_results"]);
        if let Some(summary) = summary.finish() {
            return Some(summary);
        }
    }

    if capability_matches(capability_id, "memory_write") {
        let mut summary = SummaryBuilder::default();
        if let Some(target) = safe_path_arg(value, &["target", "path"]) {
            summary.push("target", target);
        }
        if value.get("old_string").is_some() || value.get("new_string").is_some() {
            summary.push("mode", "patch");
        }
        if let Some(append) = bool_arg(value, &["append"]) {
            summary.push("append", append.to_string());
        }
        if let Some(content) = string_arg(value, &["content"]) {
            summary.push("content_bytes", content.len().to_string());
        }
        if let Some(summary) = summary.finish() {
            return Some(summary);
        }
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

#[derive(Default)]
struct SummaryBuilder {
    lines: Vec<String>,
    truncated: bool,
}

impl SummaryBuilder {
    fn push(&mut self, label: &str, value: impl Into<String>) {
        self.push_with_truncation(label, value.into(), false);
    }

    fn push_with_truncation(&mut self, label: &str, value: String, truncated: bool) {
        if value.is_empty() {
            return;
        }
        self.truncated |= truncated;
        self.lines.push(format!("{label}: {value}"));
    }

    fn finish(self) -> Option<CapabilityDisplayText> {
        if self.lines.is_empty() {
            return None;
        }
        let mut summary =
            bounded_display_text(&self.lines.join("\n"), CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES);
        summary.truncated |= self.truncated;
        Some(summary)
    }
}

fn capability_matches(capability_id: &str, short_name: &str) -> bool {
    capability_id == short_name
        || capability_id == format!("builtin.{short_name}")
        || capability_id.ends_with(&format!(".{short_name}"))
}

fn string_arg<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
        .filter(|text| !text.is_empty())
}

fn u64_arg(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_u64))
}

fn bool_arg(value: &serde_json::Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_bool))
}

fn safe_path_arg(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
        .and_then(safe_display_path)
}

fn push_text_arg(
    summary: &mut SummaryBuilder,
    value: &serde_json::Value,
    label: &str,
    keys: &[&str],
) {
    if let Some(text) = string_arg(value, keys) {
        let value = bounded_summary_value(text);
        summary.push_with_truncation(label, value.text, value.truncated);
    }
}

fn push_number_arg(
    summary: &mut SummaryBuilder,
    value: &serde_json::Value,
    label: &str,
    keys: &[&str],
) {
    if let Some(number) = u64_arg(value, keys) {
        summary.push(label, number.to_string());
    }
}

fn bounded_summary_value(text: &str) -> CapabilityDisplayText {
    bounded_display_text(text, CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES / 2)
}

fn safe_url_display(url: &str) -> CapabilityDisplayText {
    bounded_summary_value(&strip_url_sensitive_parts(url))
}

fn strip_url_sensitive_parts(url: &str) -> String {
    sanitize_url_for_display(url)
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

/// A compact, display-safe "primary argument" for the activity row's inline
/// detail — the single most salient input for a tool, so the row reads like
/// `nearai.web_search   <query>` / `shell   <command>` / `read_file   <path>`
/// instead of a bare tool name. Reuses the same sanitizing formatters as
/// `input_summary` (URL stripping, shell redaction, byte bounds) and falls back
/// to the path subtitle for tools without a recognized primary argument.
fn primary_arg_subtitle(capability_id: &str, value: &serde_json::Value) -> Option<String> {
    // Search-shaped tools → the query string.
    if (capability_matches(capability_id, "memory_search")
        || capability_matches(capability_id, "web_search")
        || capability_matches(capability_id, "search")
        || capability_matches(capability_id, "llm_context"))
        && let Some(query) = string_arg(value, &["query", "q", "text", "pattern"])
    {
        return non_empty(bounded_summary_value(query).text);
    }

    // Shell → the command (with the existing secret-redacting formatter).
    if capability_matches(capability_id, "shell")
        && let Some(command) = string_arg(value, &["command"])
    {
        return non_empty(shell_command_display_text(command).text);
    }

    // HTTP / fetch → the URL (sensitive parts stripped).
    if (capability_matches(capability_id, "http")
        || capability_matches(capability_id, "http.save")
        || capability_matches(capability_id, "web_fetch")
        || capability_matches(capability_id, "get_content"))
        && let Some(url) = string_arg(value, &["url"])
    {
        return non_empty(safe_url_display(url).text);
    }

    // Glob / grep → the pattern.
    if (capability_matches(capability_id, "glob") || capability_matches(capability_id, "grep"))
        && let Some(pattern) = string_arg(value, &["pattern"])
    {
        return non_empty(bounded_summary_value(pattern).text);
    }

    // Everything else (read_file, write_file, list_dir, apply_patch, memory_*)
    // → the path/target.
    safe_path_subtitle(value)
}

/// Returns `true` when the path contains characters that are inherently unsafe
/// to display: traversals, home-dir sigils, backslashes, or control characters.
///
/// Both subtitle validators call this before their own path-shape checks so
/// rejection rules stay in one place.
fn has_unsafe_path_chars(path: &str) -> bool {
    path.is_empty()
        || path.starts_with('~')
        || path.contains("..")
        || path.contains('\\')
        || path.chars().any(char::is_control)
}

/// Returns a safe display subtitle for input-argument paths (pending-state UI).
///
/// Strips the scoped-root prefix so only the workspace-relative portion is
/// shown. Any other absolute path is rejected.
fn safe_display_path(path: &str) -> Option<String> {
    if has_unsafe_path_chars(path) {
        return None;
    }
    let display = if let Some(rel) = path
        .strip_prefix("/workspace/")
        .or_else(|| path.strip_prefix("/project/"))
    {
        rel
    } else if path.starts_with('/') {
        return None;
    } else {
        path
    };
    Some(bounded_display_text(display, CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES).text)
}

/// Returns a safe display subtitle for output-side preview subtitles.
///
/// Keeps the full scoped path (including the `/workspace/` or `/project/`
/// prefix) because the renderer uses it as a file identifier. Rejects all
/// other absolute paths.
fn safe_preview_subtitle(subtitle: &str) -> Option<String> {
    if has_unsafe_path_chars(subtitle) {
        return None;
    }
    if subtitle.starts_with('/')
        && !subtitle.starts_with("/workspace/")
        && !subtitle.starts_with("/project/")
    {
        return None;
    }
    Some(truncate_bytes(subtitle, CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES).text)
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

fn bounded_display_text(text: &str, max_bytes: usize) -> CapabilityDisplayText {
    let sanitized = sanitize_text(text);
    truncate_bytes(&sanitized, max_bytes)
}

fn bounded_preview_text(text: &str) -> CapabilityDisplayText {
    let sanitized = sanitize_text(text);
    truncate_bytes(&sanitized, CAPABILITY_DISPLAY_PREVIEW_MAX_BYTES)
}

fn non_empty(text: String) -> Option<String> {
    if text.is_empty() { None } else { Some(text) }
}

fn truncate_bytes(text: &str, max_bytes: usize) -> CapabilityDisplayText {
    truncate_capability_display_text(text, max_bytes)
}

pub(crate) fn sanitize_text(text: &str) -> String {
    safety_sanitize_text(text)
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use serde_json::json;

    use super::*;

    #[test]
    fn record_input_recovers_from_poisoned_pending_input_lock() {
        let store = CapabilityDisplayPreviewStore::default();
        let poison_result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = store.pending.lock().expect("pending input lock");
            panic!("poison pending input lock");
        }));
        assert!(poison_result.is_err());

        let input_ref = CapabilityInputRef::new("input:poisoned-preview").expect("input ref");
        store.record_input(
            "run-poisoned-preview",
            &input_ref,
            "shell",
            &json!({ "command": "echo hi" }),
        );

        let pending = store.lock_pending_inputs();
        let input = pending
            .by_ref
            .get(input_ref.as_str())
            .expect("input preview should be recorded after poisoned lock recovery");
        assert_eq!(input.title, "shell");
        assert_eq!(input.input_summary.as_deref(), Some("command: echo hi"));
    }
}
