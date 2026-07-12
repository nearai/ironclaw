use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_product_workflow::{
    OperatorLogsService, RebornLogEntry, RebornLogLevel, RebornLogQueryRequest,
    RebornLogQueryResponse, RebornServicesError, WebUiAuthenticatedCaller,
    normalize_operator_log_context_value,
};
use ironclaw_safety::{LeakDetector, sensitive_paths::is_sensitive_path_str};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::{Event, Id};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

const HISTORY_CAP: usize = 500;
const DEFAULT_LIMIT: usize = 100;
const MAX_LOG_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_LOG_RESPONSE_BYTES: usize = 256 * 1024;
const LOG_TRUNCATED_SUFFIX: &str = " ... [truncated]";
const SOURCE: &str = "in_memory_tracing";

static OPERATOR_LOGS: LazyLock<Arc<OperatorLogBuffer>> =
    LazyLock::new(|| Arc::new(OperatorLogBuffer::new(HISTORY_CAP)));

pub struct OperatorLogLayer;

impl<S> tracing_subscriber::Layer<S> for OperatorLogLayer
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::new();
        attrs.record(&mut visitor);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut()
                .insert(SpanLogFields(visitor.take_structured_fields()));
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut visitor = MessageVisitor::new();
        values.record(&mut visitor);
        let mut extensions = span.extensions_mut();
        if let Some(existing) = extensions.get_mut::<SpanLogFields>() {
            merge_fields(&mut existing.0, visitor.take_structured_fields());
        } else {
            extensions.insert(SpanLogFields(visitor.take_structured_fields()));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);
        let mut fields = span_fields(&ctx, event);
        merge_fields(&mut fields, std::mem::take(&mut visitor.structured_fields));
        capture_tracing_log(
            metadata.level(),
            metadata.target(),
            visitor.finish(),
            fields,
        );
    }
}

#[derive(Debug, Clone)]
struct SpanLogFields(Vec<(String, String)>);

struct MessageVisitor {
    message: String,
    fields: Vec<String>,
    structured_fields: Vec<(String, String)>,
}

impl MessageVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
            fields: Vec::new(),
            structured_fields: Vec::new(),
        }
    }

    fn finish(self) -> String {
        if self.fields.is_empty() {
            self.message
        } else {
            format!("{} {}", self.message, self.fields.join(" "))
        }
    }

    fn record_value(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = value;
        } else {
            self.fields.push(format!("{}={value}", field.name()));
            if is_correlation_field(field.name()) {
                self.structured_fields
                    .push((field.name().to_string(), value));
            }
        }
    }

    fn take_structured_fields(self) -> Vec<(String, String)> {
        self.structured_fields
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let rendered = render_debug_value(value);
            self.message = rendered
        } else {
            let rendered = render_debug_value(value);
            self.fields.push(format!("{}={rendered}", field.name()));
            if is_correlation_field(field.name()) {
                self.structured_fields
                    .push((field.name().to_string(), rendered));
            }
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_value(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_value(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record_value(field, value.to_string());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push(format!("{}={value}", field.name()));
            if is_correlation_field(field.name()) {
                self.structured_fields
                    .push((field.name().to_string(), value.to_string()));
            }
        }
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.record_value(field, value.to_string());
    }
}

fn render_debug_value(value: &dyn std::fmt::Debug) -> String {
    let rendered = format!("{value:?}");
    rendered
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(rendered.as_str())
        .to_string()
}

fn is_correlation_field(name: &str) -> bool {
    matches!(
        name,
        "thread_id"
            | "run_id"
            | "turn_run_id"
            | "submitted_run_id"
            | "turn_id"
            | "submission_id"
            | "tool_call_id"
            | "invocation_id"
            | "capability_invocation_id"
            | "tool_name"
            | "capability_id"
            | "source"
            | "channel"
            | "adapter"
            | "worker"
            | "trigger_source"
    )
}

fn span_fields<S>(ctx: &Context<'_, S>, event: &Event<'_>) -> Vec<(String, String)>
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    let Some(scope) = ctx.event_scope(event) else {
        return Vec::new();
    };
    let mut fields = Vec::new();
    for span in scope.from_root() {
        if let Some(span_fields) = span.extensions().get::<SpanLogFields>() {
            merge_fields(&mut fields, span_fields.0.clone());
        }
    }
    fields
}

fn merge_fields(target: &mut Vec<(String, String)>, fields: Vec<(String, String)>) {
    for (name, value) in fields {
        if let Some(index) = target
            .iter()
            .position(|(existing_name, _)| existing_name == &name)
        {
            target.remove(index);
        }
        target.push((name, value));
    }
}

#[derive(Debug, Clone)]
struct StoredLogEntry {
    id: u64,
    timestamp: DateTime<Utc>,
    level: RebornLogLevel,
    target: String,
    message: String,
    correlation: LogCorrelation,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LogCorrelation {
    thread_id: Option<String>,
    run_id: Option<String>,
    turn_id: Option<String>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    source: Option<String>,
}

#[derive(Debug)]
struct OperatorLogState {
    next_id: u64,
    entries: VecDeque<StoredLogEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperatorLogQueryMode {
    Page,
    Tail,
    Follow,
}

impl From<&RebornLogQueryRequest> for OperatorLogQueryMode {
    fn from(request: &RebornLogQueryRequest) -> Self {
        if request.follow {
            Self::Follow
        } else if request.tail {
            Self::Tail
        } else {
            Self::Page
        }
    }
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
        self.record_with_fields(level, target, message, &[]);
    }

    fn record_with_fields(
        &self,
        level: RebornLogLevel,
        target: &str,
        message: String,
        fields: &[(String, String)],
    ) {
        let message = self
            .leak_detector
            .scan_and_clean(&message)
            .unwrap_or_else(|_| "[log message redacted: contained blocked secret]".to_string());
        let message = redact_sensitive_log_paths(&message);
        let message = truncate_utf8_with_suffix(&message, MAX_LOG_MESSAGE_BYTES);
        let correlation = self.correlation_from_fields(fields);
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
            correlation,
        });
    }

    fn correlation_from_fields(&self, fields: &[(String, String)]) -> LogCorrelation {
        let mut correlation = LogCorrelation::default();
        for (name, value) in fields.iter().rev() {
            let cleaned = self.sanitize_context_value(value);
            match name.as_str() {
                "thread_id" => set_first(&mut correlation.thread_id, cleaned),
                "run_id" | "turn_run_id" | "submitted_run_id" => {
                    set_first(&mut correlation.run_id, cleaned)
                }
                "turn_id" | "submission_id" => set_first(&mut correlation.turn_id, cleaned),
                "tool_call_id" | "invocation_id" | "capability_invocation_id" => {
                    set_first(&mut correlation.tool_call_id, cleaned)
                }
                "tool_name" | "capability_id" => set_first(&mut correlation.tool_name, cleaned),
                "source" | "channel" | "adapter" | "worker" | "trigger_source" => {
                    set_first(&mut correlation.source, cleaned)
                }
                _ => {}
            }
        }
        correlation
    }

    fn sanitize_context_value(&self, value: &str) -> String {
        let cleaned = self
            .leak_detector
            .scan_and_clean(value)
            .unwrap_or_else(|_| "[redacted]".to_string());
        normalize_operator_log_context_value(&cleaned)
    }

    fn normalize_query_request(&self, mut request: RebornLogQueryRequest) -> RebornLogQueryRequest {
        request.thread_id = request
            .thread_id
            .as_deref()
            .map(|value| self.sanitize_context_value(value));
        request.run_id = request
            .run_id
            .as_deref()
            .map(|value| self.sanitize_context_value(value));
        request.turn_id = request
            .turn_id
            .as_deref()
            .map(|value| self.sanitize_context_value(value));
        request.tool_call_id = request
            .tool_call_id
            .as_deref()
            .map(|value| self.sanitize_context_value(value));
        request.tool_name = request
            .tool_name
            .as_deref()
            .map(|value| self.sanitize_context_value(value));
        request.source = request
            .source
            .as_deref()
            .map(|value| self.sanitize_context_value(value));
        request
    }

    fn query(&self, request: RebornLogQueryRequest) -> RebornLogQueryResponse {
        let limit = request
            .limit
            .map(|value| value as usize)
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(1, self.capacity);
        let request = self.normalize_query_request(request);
        let mode = OperatorLogQueryMode::from(&request);
        let before_id = request.cursor.as_deref().and_then(parse_before_cursor);
        let after_id = request.cursor.as_deref().and_then(parse_after_cursor);
        let target_filter = request.target.as_ref().map(|target| target.to_lowercase());
        let Ok(state) = self.state.lock() else {
            return RebornLogQueryResponse {
                source: SOURCE.to_string(),
                entries: Vec::new(),
                next_cursor: None,
                tail_supported: true,
                follow_supported: true,
            };
        };

        let mut selected = Vec::with_capacity(limit.min(self.capacity));
        let mut selected_bytes = 0usize;
        let mut next_cursor = None;

        match mode {
            OperatorLogQueryMode::Follow => {
                let start_after_id = after_id.unwrap_or_else(|| state.next_id.saturating_sub(1));
                let mut last_selected_id = None;
                for entry in state.entries.iter() {
                    if entry.id <= start_after_id {
                        continue;
                    }
                    if !entry_matches_request(entry, &request, target_filter.as_deref()) {
                        continue;
                    }

                    if selected.len() >= limit {
                        break;
                    }
                    let api_entry = RebornLogEntry::from(entry.clone());
                    let entry_bytes = response_entry_bytes(&api_entry);
                    let next_selected_bytes = response_entries_bytes_after_push(
                        selected_bytes,
                        selected.len(),
                        entry_bytes,
                    );
                    if !selected.is_empty() && next_selected_bytes > MAX_LOG_RESPONSE_BYTES {
                        break;
                    }
                    last_selected_id = Some(entry.id);
                    selected_bytes = next_selected_bytes;
                    selected.push(api_entry);
                }
                next_cursor = Some(after_cursor(last_selected_id.unwrap_or(start_after_id)));
            }
            OperatorLogQueryMode::Tail => {
                for entry in state.entries.iter().rev() {
                    if !entry_matches_request(entry, &request, target_filter.as_deref()) {
                        continue;
                    }

                    if selected.len() >= limit {
                        break;
                    }
                    let api_entry = RebornLogEntry::from(entry.clone());
                    let entry_bytes = response_entry_bytes(&api_entry);
                    let next_selected_bytes = response_entries_bytes_after_push(
                        selected_bytes,
                        selected.len(),
                        entry_bytes,
                    );
                    if !selected.is_empty() && next_selected_bytes > MAX_LOG_RESPONSE_BYTES {
                        break;
                    }
                    selected_bytes = next_selected_bytes;
                    selected.push(api_entry);
                }
                selected.reverse();
                next_cursor = Some(after_cursor(state.next_id.saturating_sub(1)));
            }
            OperatorLogQueryMode::Page => {
                for entry in state.entries.iter().rev() {
                    if before_id.is_some_and(|id| entry.id >= id) {
                        continue;
                    }
                    if !entry_matches_request(entry, &request, target_filter.as_deref()) {
                        continue;
                    }

                    if selected.len() >= limit {
                        next_cursor = selected
                            .last()
                            .map(|entry: &RebornLogEntry| format!("before:{}", entry.id));
                        break;
                    }
                    let api_entry = RebornLogEntry::from(entry.clone());
                    let entry_bytes = response_entry_bytes(&api_entry);
                    let next_selected_bytes = response_entries_bytes_after_push(
                        selected_bytes,
                        selected.len(),
                        entry_bytes,
                    );
                    if !selected.is_empty() && next_selected_bytes > MAX_LOG_RESPONSE_BYTES {
                        next_cursor = selected
                            .last()
                            .map(|entry: &RebornLogEntry| format!("before:{}", entry.id));
                        break;
                    }
                    selected_bytes = next_selected_bytes;
                    selected.push(api_entry);
                }
            }
        }

        RebornLogQueryResponse {
            source: SOURCE.to_string(),
            entries: selected,
            next_cursor,
            tail_supported: true,
            follow_supported: true,
        }
    }
}

fn set_first(slot: &mut Option<String>, value: String) {
    if slot.is_none() {
        *slot = Some(value);
    }
}

fn response_entry_bytes(entry: &RebornLogEntry) -> usize {
    serde_json::to_vec(entry).map_or(usize::MAX, |bytes| bytes.len())
}

fn response_entries_bytes_after_push(
    current_bytes: usize,
    selected_len: usize,
    entry_bytes: usize,
) -> usize {
    current_bytes
        .saturating_add(entry_bytes)
        .saturating_add(if selected_len == 0 { 2 } else { 1 })
}

fn entry_matches_request(
    entry: &StoredLogEntry,
    request: &RebornLogQueryRequest,
    target_filter: Option<&str>,
) -> bool {
    if request.level.is_some_and(|level| entry.level != level) {
        return false;
    }
    if let Some(target) = target_filter
        && !entry.target.to_lowercase().contains(target)
    {
        return false;
    }
    entry.matches_query(request)
}

impl StoredLogEntry {
    fn matches_query(&self, request: &RebornLogQueryRequest) -> bool {
        query_value_matches(&self.correlation.thread_id, &request.thread_id)
            && query_value_matches(&self.correlation.run_id, &request.run_id)
            && query_value_matches(&self.correlation.turn_id, &request.turn_id)
            && query_value_matches(&self.correlation.tool_call_id, &request.tool_call_id)
            && query_value_matches(&self.correlation.tool_name, &request.tool_name)
            && query_value_matches(&self.correlation.source, &request.source)
    }
}

fn query_value_matches(entry_value: &Option<String>, request_value: &Option<String>) -> bool {
    match request_value {
        Some(expected) => entry_value.as_deref() == Some(expected.as_str()),
        None => true,
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
            thread_id: entry.correlation.thread_id,
            run_id: entry.correlation.run_id,
            turn_id: entry.correlation.turn_id,
            tool_call_id: entry.correlation.tool_call_id,
            tool_name: entry.correlation.tool_name,
            source: entry.correlation.source,
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

pub fn capture_tracing_log(
    level: &tracing::Level,
    target: &str,
    message: String,
    fields: Vec<(String, String)>,
) {
    operator_log_buffer().record_with_fields(
        reborn_level_from_tracing(level),
        target,
        message,
        &fields,
    );
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

fn parse_after_cursor(cursor: &str) -> Option<u64> {
    cursor
        .strip_prefix("after:")
        .and_then(|value| value.parse::<u64>().ok())
}

fn after_cursor(id: u64) -> String {
    format!("after:{id}")
}

fn redact_sensitive_log_paths(value: &str) -> String {
    let mut redacted = String::with_capacity(value.len());
    for segment in value.split_inclusive(char::is_whitespace) {
        let token = segment.trim_end_matches(char::is_whitespace);
        let whitespace = &segment[token.len()..];
        redacted.push_str(&redact_sensitive_log_path_token(token));
        redacted.push_str(whitespace);
    }
    redacted
}

fn redact_sensitive_log_path_token(token: &str) -> Cow<'_, str> {
    let mut cursor = 0usize;
    let mut redacted = String::new();
    let mut last_copied = 0usize;
    let mut changed = false;

    while let Some(offset) = token[cursor..].find(['/', '\\']) {
        let separator = cursor + offset;
        let start = sensitive_path_candidate_start(token, separator);
        let end = sensitive_path_candidate_end(token, start);
        if start < end && is_sensitive_log_path_candidate(&token[start..end]) {
            if !changed {
                redacted = String::with_capacity(token.len());
                changed = true;
            }
            redacted.push_str(&token[last_copied..start]);
            redacted.push_str("[REDACTED_PATH]");
            last_copied = end;
            cursor = end;
        } else {
            cursor = token[separator..]
                .chars()
                .next()
                .map_or(token.len(), |character| separator + character.len_utf8());
        }
    }

    if changed {
        redacted.push_str(&token[last_copied..]);
        Cow::Owned(redacted)
    } else {
        Cow::Borrowed(token)
    }
}

fn sensitive_path_candidate_start(token: &str, separator: usize) -> usize {
    if separator >= 2 {
        let prefix = &token[..separator];
        let mut chars = prefix.chars().rev();
        if chars.next() == Some(':')
            && chars
                .next()
                .is_some_and(|character| character.is_ascii_alphabetic())
        {
            return separator - 2;
        }
    }

    token[..separator]
        .char_indices()
        .rev()
        .find_map(|(offset, character)| {
            if is_log_path_boundary(character) {
                Some(offset + character.len_utf8())
            } else {
                None
            }
        })
        .unwrap_or(0)
}

fn sensitive_path_candidate_end(token: &str, start: usize) -> usize {
    token[start..]
        .char_indices()
        .find_map(|(offset, character)| {
            if character == ':'
                && offset == 1
                && token[start..]
                    .chars()
                    .next()
                    .is_some_and(|prefix| prefix.is_ascii_alphabetic())
            {
                return None;
            }
            if is_log_path_boundary(character) {
                Some(start + offset)
            } else {
                None
            }
        })
        .unwrap_or(token.len())
}

fn is_log_path_boundary(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '`' | ',' | ';' | ':' | ')' | '(' | ']' | '[' | '{' | '}' | '='
    )
}

fn is_sensitive_log_path_candidate(candidate: &str) -> bool {
    if is_sensitive_path_str(candidate) {
        return true;
    }

    let mut previous = None;
    let mut has_operator_token_path = false;
    let mut has_secret_segment = false;
    let mut has_credential_filename = false;
    for segment in candidate
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
    {
        if segment.eq_ignore_ascii_case("secret") || segment.eq_ignore_ascii_case("secrets") {
            has_secret_segment = true;
        }
        if contains_ascii_case_insensitive(segment, "token")
            || contains_ascii_case_insensitive(segment, "credential")
            || contains_ascii_case_insensitive(segment, "secret")
            || ends_with_ascii_case_insensitive(segment, ".key")
        {
            has_credential_filename = true;
        }
        if previous == Some("reborn") && contains_ascii_case_insensitive(segment, "operator-token")
        {
            has_operator_token_path = true;
        }
        previous = if segment.eq_ignore_ascii_case(".ironclaw")
            || (previous == Some(".ironclaw") && segment.eq_ignore_ascii_case("reborn"))
        {
            Some(if segment.eq_ignore_ascii_case(".ironclaw") {
                ".ironclaw"
            } else {
                "reborn"
            })
        } else {
            None
        };
    }

    has_operator_token_path || (has_secret_segment && has_credential_filename)
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn ends_with_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .get(haystack.len().saturating_sub(needle.len())..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(needle.as_bytes()))
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
    use tracing_subscriber::prelude::*;

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

        let first = buffer.query(RebornLogQueryRequest::default().set_limit(2));
        assert_eq!(
            first
                .entries
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>(),
            vec!["message 4", "message 3"]
        );
        let cursor = first.next_cursor.expect("older page cursor");

        let second = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(2)
                .set_cursor(cursor),
        );
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
    fn query_evicts_oldest_entries_when_capacity_is_reached() {
        let buffer = OperatorLogBuffer::new(3);
        for index in 0..5 {
            buffer.record(
                RebornLogLevel::Info,
                "ironclaw::test",
                format!("message {index}"),
            );
        }

        let response = buffer.query(RebornLogQueryRequest::default().set_limit(10));

        assert_eq!(
            response
                .entries
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>(),
            vec!["message 4", "message 3", "message 2"]
        );
        assert!(response.next_cursor.is_none());
    }

    #[test]
    fn invalid_page_cursor_behaves_like_no_cursor() {
        let buffer = OperatorLogBuffer::new(10);
        for index in 0..3 {
            buffer.record(
                RebornLogLevel::Info,
                "ironclaw::test",
                format!("message {index}"),
            );
        }

        let response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(2)
                .set_cursor("before:not-a-number"),
        );

        assert_eq!(
            response
                .entries
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>(),
            vec!["message 2", "message 1"]
        );
        assert_eq!(response.next_cursor.as_deref(), Some("before:2"));
    }

    #[test]
    fn query_filters_by_level_and_target() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(RebornLogLevel::Info, "ironclaw::alpha", "alpha".to_string());
        buffer.record(RebornLogLevel::Warn, "ironclaw::beta", "beta".to_string());
        buffer.record(RebornLogLevel::Warn, "other::beta", "other".to_string());

        let response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_level(RebornLogLevel::Warn)
                .set_target("ironclaw"),
        );

        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].message, "beta");
    }

    #[test]
    fn query_target_filter_is_case_insensitive_contains() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(
            RebornLogLevel::Info,
            "IronClaw::OperatorLogs",
            "mixed target".to_string(),
        );
        buffer.record(
            RebornLogLevel::Info,
            "other::target",
            "other target".to_string(),
        );

        let response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_target("operatorlogs"),
        );

        assert_eq!(
            response
                .entries
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>(),
            vec!["mixed target"]
        );
    }

    #[test]
    fn query_filters_by_structured_run_and_thread_fields() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record_with_fields(
            RebornLogLevel::Info,
            "ironclaw::run",
            "thread a run a".to_string(),
            &[
                ("thread_id".to_string(), "thread-a".to_string()),
                ("run_id".to_string(), "run-a".to_string()),
                ("invocation_id".to_string(), "tool-call-a".to_string()),
                ("capability_id".to_string(), "shell".to_string()),
                ("channel".to_string(), "slack".to_string()),
            ],
        );
        buffer.record_with_fields(
            RebornLogLevel::Info,
            "ironclaw::run",
            "thread b run b".to_string(),
            &[
                ("thread_id".to_string(), "thread-b".to_string()),
                ("run_id".to_string(), "run-b".to_string()),
                ("invocation_id".to_string(), "tool-call-b".to_string()),
                ("source".to_string(), "webui".to_string()),
            ],
        );

        let response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_thread_id("thread-a")
                .set_run_id("run-a")
                .set_tool_call_id("tool-call-a")
                .set_tool_name("shell")
                .set_source("slack"),
        );

        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].message, "thread a run a");
        assert_eq!(response.entries[0].thread_id.as_deref(), Some("thread-a"));
        assert_eq!(response.entries[0].run_id.as_deref(), Some("run-a"));
        assert_eq!(
            response.entries[0].tool_call_id.as_deref(),
            Some("tool-call-a")
        );
        assert_eq!(response.entries[0].tool_name.as_deref(), Some("shell"));
        assert_eq!(response.entries[0].source.as_deref(), Some("slack"));
    }

    #[test]
    fn query_combines_scope_with_level_and_target_filters() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record_with_fields(
            RebornLogLevel::Info,
            "ironclaw::worker",
            "wrong level".to_string(),
            &[("run_id".to_string(), "run-a".to_string())],
        );
        buffer.record_with_fields(
            RebornLogLevel::Warn,
            "other::worker",
            "wrong target".to_string(),
            &[("run_id".to_string(), "run-a".to_string())],
        );
        buffer.record_with_fields(
            RebornLogLevel::Warn,
            "ironclaw::worker",
            "match".to_string(),
            &[("run_id".to_string(), "run-a".to_string())],
        );

        let response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_level(RebornLogLevel::Warn)
                .set_target("ironclaw")
                .set_run_id("run-a"),
        );

        assert_eq!(
            response
                .entries
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>(),
            vec!["match"]
        );
    }

    #[test]
    fn correlation_prefers_inner_alias_fields() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record_with_fields(
            RebornLogLevel::Info,
            "ironclaw::run",
            "inner wins".to_string(),
            &[
                ("run_id".to_string(), "outer-run".to_string()),
                ("turn_run_id".to_string(), "inner-run".to_string()),
                ("source".to_string(), "outer-source".to_string()),
                ("channel".to_string(), "inner-source".to_string()),
            ],
        );

        let response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_run_id("inner-run")
                .set_source("inner-source"),
        );

        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].run_id.as_deref(), Some("inner-run"));
        assert_eq!(response.entries[0].source.as_deref(), Some("inner-source"));
    }

    #[test]
    fn query_normalizes_overlong_scope_filter_like_stored_context() {
        let buffer = OperatorLogBuffer::new(10);
        let run_id = format!("{}😀tail", "run-".repeat(70));
        buffer.record_with_fields(
            RebornLogLevel::Info,
            "ironclaw::run",
            "overlong run id".to_string(),
            &[("run_id".to_string(), run_id.clone())],
        );

        let response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_run_id(run_id),
        );

        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].message, "overlong run id");
        assert!(
            response.entries[0]
                .run_id
                .as_deref()
                .is_some_and(|value| value.ends_with(LOG_TRUNCATED_SUFFIX))
        );
    }

    #[test]
    fn operator_log_layer_records_span_event_correlation_fields() {
        let token = uuid::Uuid::new_v4().to_string();
        let thread_id = format!("thread-{token}");
        let run_id = format!("run-{token}");
        let turn_id = format!("turn-{token}");
        let tool_call_id = format!("tool-call-{token}");
        let source = "slack";
        let tool_name = "shell";
        let subscriber = tracing_subscriber::registry().with(OperatorLogLayer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            let outer =
                tracing::info_span!("outer", thread_id = thread_id.as_str(), source = source);
            let _outer_guard = outer.enter();
            let inner = tracing::info_span!(
                "inner",
                run_id = run_id.as_str(),
                tool_call_id = tool_call_id.as_str()
            );
            let _inner_guard = inner.enter();

            tracing::info!(
                target: "ironclaw::operator_logs_layer_test",
                turn_id = turn_id.as_str(),
                tool_name = tool_name,
                "layer path"
            );
        });

        let response = operator_log_buffer().query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_thread_id(thread_id.clone())
                .set_run_id(run_id.clone())
                .set_turn_id(turn_id.clone())
                .set_tool_call_id(tool_call_id.clone())
                .set_tool_name(tool_name)
                .set_source(source),
        );

        assert_eq!(response.entries.len(), 1);
        let message = &response.entries[0].message;
        let expected_turn_field = format!("turn_id={turn_id}");
        let expected_tool_field = format!("tool_name={tool_name}");
        assert!(message.starts_with("layer path"));
        assert!(message.contains(&expected_turn_field));
        assert!(message.contains(&expected_tool_field));
        assert_eq!(
            response.entries[0].thread_id.as_deref(),
            Some(thread_id.as_str())
        );
        assert_eq!(response.entries[0].run_id.as_deref(), Some(run_id.as_str()));
        assert_eq!(
            response.entries[0].turn_id.as_deref(),
            Some(turn_id.as_str())
        );
        assert_eq!(
            response.entries[0].tool_call_id.as_deref(),
            Some(tool_call_id.as_str())
        );
        assert_eq!(response.entries[0].tool_name.as_deref(), Some(tool_name));
        assert_eq!(response.entries[0].source.as_deref(), Some(source));
    }

    #[test]
    fn operator_log_layer_prefers_event_canonical_field_over_span_alias() {
        let token = uuid::Uuid::new_v4().to_string();
        let span_run_id = format!("span-run-{token}");
        let event_run_id = format!("event-run-{token}");
        let subscriber = tracing_subscriber::registry().with(OperatorLogLayer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            let span = tracing::info_span!("run context", turn_run_id = span_run_id.as_str());
            let _guard = span.enter();
            tracing::info!(
                target: "ironclaw::operator_logs_layer_test",
                run_id = event_run_id.as_str(),
                "event canonical run"
            );
        });

        let response = operator_log_buffer().query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_run_id(event_run_id.clone()),
        );

        assert_eq!(response.entries.len(), 1);
        assert_eq!(
            response.entries[0].run_id.as_deref(),
            Some(event_run_id.as_str())
        );
    }

    #[test]
    fn operator_log_layer_keeps_arbitrary_event_fields_out_of_correlation_storage() {
        let token = uuid::Uuid::new_v4().to_string();
        let run_id = format!("run-{token}");
        let subscriber = tracing_subscriber::registry().with(OperatorLogLayer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            let span = tracing::info_span!(
                "long-lived context",
                payload = "not-correlation",
                run_id = run_id.as_str()
            );
            let _guard = span.enter();
            tracing::info!(
                target: "ironclaw::operator_logs_layer_test",
                payload = "rendered-event-field",
                "filtered structured field"
            );
        });

        let response = operator_log_buffer().query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_run_id(run_id.clone()),
        );

        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].run_id.as_deref(), Some(run_id.as_str()));
        assert!(response.entries[0].message.contains("rendered-event-field"));
    }

    #[test]
    fn query_paginates_after_scoped_filtering() {
        let buffer = OperatorLogBuffer::new(10);
        for index in 0..6 {
            let thread_id = if index % 2 == 0 {
                "thread-a"
            } else {
                "thread-b"
            };
            buffer.record_with_fields(
                RebornLogLevel::Info,
                "ironclaw::test",
                format!("message {index}"),
                &[("thread_id".to_string(), thread_id.to_string())],
            );
        }

        let first = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(2)
                .set_thread_id("thread-a"),
        );
        assert_eq!(
            first
                .entries
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>(),
            vec!["message 4", "message 2"]
        );
        let cursor = first.next_cursor.expect("older scoped page cursor");

        let second = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(2)
                .set_cursor(cursor)
                .set_thread_id("thread-a"),
        );
        assert_eq!(
            second
                .entries
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>(),
            vec!["message 0"]
        );
        assert!(second.next_cursor.is_none());
    }

    #[test]
    fn scoped_query_excludes_entries_without_correlation_fields() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(
            RebornLogLevel::Info,
            "ironclaw::test",
            "unscoped".to_string(),
        );
        buffer.record_with_fields(
            RebornLogLevel::Info,
            "ironclaw::test",
            "scoped".to_string(),
            &[("thread_id".to_string(), "thread-a".to_string())],
        );

        let response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_thread_id("thread-a"),
        );

        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].message, "scoped");
    }

    #[test]
    fn record_redacts_secret_shaped_messages() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(
            RebornLogLevel::Info,
            "ironclaw::test",
            "token sk-proj-test1234567890abcdefghij".to_string(),
        );

        let response = buffer.query(RebornLogQueryRequest::default().set_limit(1));

        assert_eq!(
            response.entries[0].message,
            "[log message redacted: contained blocked secret]"
        );
    }

    #[test]
    fn record_redacts_sensitive_host_paths() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(
            RebornLogLevel::Warn,
            "ironclaw::test",
            "failed to read /home/user/.config/gh/hosts.yml, retrying".to_string(),
        );

        let response = buffer.query(RebornLogQueryRequest::default().set_limit(1));

        assert_eq!(
            response.entries[0].message,
            "failed to read [REDACTED_PATH], retrying"
        );
    }

    #[test]
    fn record_redacts_sensitive_host_paths_embedded_after_prefixes() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(
            RebornLogLevel::Warn,
            "ironclaw::test",
            "path=/home/user/.config/gh/hosts.yml json={\"path\":\"/home/user/.ssh/id_ed25519\"}"
                .to_string(),
        );

        let response = buffer.query(RebornLogQueryRequest::default().set_limit(1));

        assert_eq!(
            response.entries[0].message,
            "path=[REDACTED_PATH] json={\"path\":\"[REDACTED_PATH]\"}"
        );
    }

    #[test]
    fn record_redacts_backslash_delimited_sensitive_host_paths() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(
            RebornLogLevel::Warn,
            "ironclaw::test",
            r#"win=C:\Users\alice\.ironclaw\reborn\operator-token.txt escaped=secret\token"#
                .to_string(),
        );

        let response = buffer.query(RebornLogQueryRequest::default().set_limit(1));

        assert_eq!(
            response.entries[0].message,
            "win=[REDACTED_PATH] escaped=[REDACTED_PATH]"
        );
    }

    #[test]
    fn tail_returns_latest_entries_chronologically_with_follow_cursor() {
        let buffer = OperatorLogBuffer::new(10);
        for index in 0..4 {
            buffer.record(
                RebornLogLevel::Info,
                "ironclaw::test",
                format!("message {index}"),
            );
        }

        let response = buffer.query(RebornLogQueryRequest::default().set_limit(2).set_tail(true));

        assert_eq!(
            response
                .entries
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>(),
            vec!["message 2", "message 3"]
        );
        assert!(response.tail_supported);
        assert!(response.follow_supported);
        assert_eq!(response.next_cursor.as_deref(), Some("after:4"));
    }

    #[test]
    fn follow_returns_entries_after_cursor_chronologically() {
        let buffer = OperatorLogBuffer::new(10);
        for index in 0..3 {
            buffer.record(
                RebornLogLevel::Info,
                "ironclaw::test",
                format!("message {index}"),
            );
        }
        let tail = buffer.query(RebornLogQueryRequest::default().set_limit(2).set_tail(true));
        let cursor = tail.next_cursor.expect("follow cursor");
        buffer.record(
            RebornLogLevel::Warn,
            "ironclaw::test",
            "message 3".to_string(),
        );
        buffer.record(
            RebornLogLevel::Warn,
            "ironclaw::test",
            "message 4".to_string(),
        );

        let response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_cursor(cursor)
                .set_follow(true),
        );

        assert_eq!(
            response
                .entries
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>(),
            vec!["message 3", "message 4"]
        );
        assert_eq!(response.next_cursor.as_deref(), Some("after:5"));
    }

    #[test]
    fn follow_without_cursor_starts_after_current_end() {
        let buffer = OperatorLogBuffer::new(10);
        for index in 0..3 {
            buffer.record(
                RebornLogLevel::Info,
                "ironclaw::test",
                format!("message {index}"),
            );
        }

        let response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_follow(true),
        );

        assert!(response.entries.is_empty());
        assert!(response.tail_supported);
        assert!(response.follow_supported);
        assert_eq!(response.next_cursor.as_deref(), Some("after:3"));
    }

    #[test]
    fn filtered_tail_cursor_uses_highest_retained_log_id() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(RebornLogLevel::Warn, "ironclaw::test", "match".to_string());
        buffer.record(
            RebornLogLevel::Info,
            "ironclaw::test",
            "newer non-match".to_string(),
        );

        let response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_level(RebornLogLevel::Warn)
                .set_tail(true),
        );

        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].message, "match");
        assert_eq!(response.next_cursor.as_deref(), Some("after:2"));
    }

    #[test]
    fn filtered_follow_cursor_stays_at_last_returned_match() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(RebornLogLevel::Info, "ironclaw::test", "base".to_string());
        buffer.record(RebornLogLevel::Warn, "ironclaw::test", "match".to_string());
        buffer.record(
            RebornLogLevel::Info,
            "ironclaw::test",
            "newer non-match".to_string(),
        );

        let response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_cursor("after:1")
                .set_level(RebornLogLevel::Warn)
                .set_follow(true),
        );

        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].message, "match");
        assert_eq!(response.next_cursor.as_deref(), Some("after:2"));
    }

    #[test]
    fn filtered_follow_cursor_allows_later_filter_changes() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(RebornLogLevel::Info, "ironclaw::test", "base".to_string());
        buffer.record(
            RebornLogLevel::Warn,
            "ironclaw::test",
            "warn match".to_string(),
        );
        buffer.record(
            RebornLogLevel::Info,
            "ironclaw::test",
            "info match".to_string(),
        );

        let warn_response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_cursor("after:1")
                .set_level(RebornLogLevel::Warn)
                .set_follow(true),
        );

        let info_response = buffer.query(
            RebornLogQueryRequest::default()
                .set_limit(10)
                .set_cursor(warn_response.next_cursor.expect("warn follow cursor"))
                .set_level(RebornLogLevel::Info)
                .set_follow(true),
        );

        assert_eq!(info_response.entries.len(), 1);
        assert_eq!(info_response.entries[0].message, "info match");
        assert_eq!(info_response.next_cursor.as_deref(), Some("after:3"));
    }

    #[test]
    fn record_truncates_large_messages_on_utf8_boundary() {
        let buffer = OperatorLogBuffer::new(10);
        buffer.record(
            RebornLogLevel::Info,
            "ironclaw::test",
            "\u{1F600}".repeat(MAX_LOG_MESSAGE_BYTES),
        );

        let response = buffer.query(RebornLogQueryRequest::default().set_limit(1));

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

        let response = buffer.query(RebornLogQueryRequest::default().set_limit(100));

        let message_bytes = response
            .entries
            .iter()
            .map(|entry| entry.message.len())
            .sum::<usize>();
        assert!(message_bytes <= MAX_LOG_RESPONSE_BYTES);
        assert!(
            serde_json::to_vec(&response.entries)
                .expect("serialize entries")
                .len()
                <= MAX_LOG_RESPONSE_BYTES
        );
        assert!(response.entries.len() < 40);
        assert!(response.next_cursor.is_some());
    }

    #[test]
    fn response_entry_bytes_counts_serialized_correlation_key_overhead() {
        let entry = StoredLogEntry {
            id: 1,
            timestamp: Utc::now(),
            level: RebornLogLevel::Info,
            target: "ironclaw::test".to_string(),
            message: "message".to_string(),
            correlation: LogCorrelation {
                thread_id: Some("thread-a".to_string()),
                run_id: Some("run-a".to_string()),
                turn_id: Some("turn-a".to_string()),
                tool_call_id: Some("tool-call-a".to_string()),
                tool_name: Some("shell".to_string()),
                source: Some("webui".to_string()),
            },
        };
        let api_entry = RebornLogEntry::from(entry.clone());

        assert_eq!(
            response_entry_bytes(&api_entry),
            serde_json::to_vec(&api_entry)
                .expect("serialize entry")
                .len()
        );
    }

    #[test]
    fn response_entries_bytes_after_push_counts_json_array_overhead() {
        let first = RebornLogEntry::from(StoredLogEntry {
            id: 1,
            timestamp: Utc::now(),
            level: RebornLogLevel::Info,
            target: "ironclaw::test".to_string(),
            message: "first".to_string(),
            correlation: LogCorrelation::default(),
        });
        let second = RebornLogEntry::from(StoredLogEntry {
            id: 2,
            timestamp: Utc::now(),
            level: RebornLogLevel::Warn,
            target: "ironclaw::test".to_string(),
            message: "second".to_string(),
            correlation: LogCorrelation::default(),
        });

        let mut selected_bytes = 0;
        selected_bytes =
            response_entries_bytes_after_push(selected_bytes, 0, response_entry_bytes(&first));
        selected_bytes =
            response_entries_bytes_after_push(selected_bytes, 1, response_entry_bytes(&second));

        assert_eq!(
            selected_bytes,
            serde_json::to_vec(&vec![first, second])
                .expect("serialize entries")
                .len()
        );
    }
}
