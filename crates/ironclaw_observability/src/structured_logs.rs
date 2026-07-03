use std::collections::BTreeMap;
use std::io::{self, Write};

use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

type LogLineWriter = fn(&str);

#[derive(Clone)]
pub struct StructuredJsonLogLayer {
    metadata: StructuredLogMetadata,
    write_line: LogLineWriter,
}

impl StructuredJsonLogLayer {
    pub fn new(default_service: &str) -> Self {
        Self {
            metadata: StructuredLogMetadata::from_env(default_service),
            write_line: write_stderr_line,
        }
    }

    #[cfg(test)]
    fn with_line_writer(default_service: &str, write_line: LogLineWriter) -> Self {
        Self {
            metadata: StructuredLogMetadata::from_env(default_service),
            write_line,
        }
    }
}

impl<S> tracing_subscriber::Layer<S> for StructuredJsonLogLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut visitor = StructuredFieldsVisitor::default();
        attrs.record(&mut visitor);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(StructuredSpanFields {
                promoted: visitor.promoted,
                fields: visitor.fields,
            });
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut visitor = StructuredFieldsVisitor::default();
        values.record(&mut visitor);
        let mut extensions = span.extensions_mut();
        if let Some(existing) = extensions.get_mut::<StructuredSpanFields>() {
            merge_json_map(&mut existing.promoted, visitor.promoted);
            merge_json_map(&mut existing.fields, visitor.fields);
        } else {
            extensions.insert(StructuredSpanFields {
                promoted: visitor.promoted,
                fields: visitor.fields,
            });
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut promoted = BTreeMap::new();
        let mut fields = BTreeMap::new();
        for span in ctx
            .event_scope(event)
            .into_iter()
            .flat_map(|scope| scope.from_root())
        {
            if let Some(span_fields) = span.extensions().get::<StructuredSpanFields>() {
                merge_json_map(&mut promoted, span_fields.promoted.clone());
                merge_json_map(&mut fields, span_fields.fields.clone());
            }
        }

        let mut visitor = StructuredFieldsVisitor::default();
        event.record(&mut visitor);
        merge_json_map(&mut promoted, visitor.promoted);
        merge_json_map(&mut fields, visitor.fields);

        let line = structured_log_line(
            &self.metadata,
            event.metadata().level(),
            event.metadata().target(),
            visitor.message,
            promoted,
            fields,
        );
        if let Ok(line) = serde_json::to_string(&Value::Object(line)) {
            (self.write_line)(&line);
        }
    }
}

fn write_stderr_line(line: &str) {
    let _ = writeln!(io::stderr(), "{line}");
}

#[derive(Clone)]
struct StructuredLogMetadata {
    service: String,
    environment: Option<String>,
    deployment_id: Option<String>,
    replica_id: Option<String>,
    git_sha: Option<String>,
}

impl StructuredLogMetadata {
    fn from_env(default_service: &str) -> Self {
        Self {
            service: first_env(&["RAILWAY_SERVICE_NAME", "IRONCLAW_SERVICE_NAME"])
                .unwrap_or_else(|| default_service.to_string()),
            environment: first_env(&[
                "RAILWAY_ENVIRONMENT_NAME",
                "IRONCLAW_ENV",
                "APP_ENV",
                "ENVIRONMENT",
            ]),
            deployment_id: first_env(&["RAILWAY_DEPLOYMENT_ID"]),
            replica_id: first_env(&["RAILWAY_REPLICA_ID"]),
            git_sha: first_env(&["RAILWAY_GIT_COMMIT_SHA", "GIT_SHA", "COMMIT_SHA"]),
        }
    }
}

fn first_env(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = std::env::var(key).ok()?;
        (!value.trim().is_empty()).then_some(value)
    })
}

#[derive(Clone)]
struct StructuredSpanFields {
    promoted: BTreeMap<String, Value>,
    fields: BTreeMap<String, Value>,
}

#[derive(Default)]
struct StructuredFieldsVisitor {
    message: String,
    promoted: BTreeMap<String, Value>,
    fields: BTreeMap<String, Value>,
}

impl StructuredFieldsVisitor {
    fn record_value(&mut self, field: &Field, value: Value) {
        if field.name() == "message" {
            self.message = json_value_to_log_string(value);
        } else if let Some(name) = canonical_log_field(field.name()) {
            self.promoted
                .insert(name.to_string(), normalize_promoted_value(name, value));
        } else {
            self.fields.insert(field.name().to_string(), value);
        }
    }
}

impl Visit for StructuredFieldsVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record_value(field, Value::String(render_debug_value(value)));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_value(field, Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_value(field, Value::Number(value.into()));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record_value(field, Value::Bool(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_value(field, Value::String(value.to_string()));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.record_value(field, Value::String(value.to_string()));
    }
}

fn canonical_log_field(name: &str) -> Option<&'static str> {
    match name {
        "thread_id" => Some("thread_id"),
        "run_id" | "turn_run_id" | "submitted_run_id" => Some("run_id"),
        "turn_id" | "submission_id" => Some("turn_id"),
        "tool_name" | "capability_id" => Some("tool_name"),
        "tool_call_id" | "invocation_id" | "capability_invocation_id" => Some("tool_call_id"),
        "request_id" | "trace_id" => Some("request_id"),
        "error_kind" => Some("error_kind"),
        "duration_ms" => Some("duration_ms"),
        _ => None,
    }
}

fn normalize_promoted_value(name: &str, value: Value) -> Value {
    if name == "duration_ms" {
        match value {
            Value::String(value) => value
                .parse::<u64>()
                .ok()
                .map(|value| Value::Number(value.into()))
                .unwrap_or(Value::String(value)),
            value => value,
        }
    } else {
        value
    }
}

fn structured_log_line(
    metadata: &StructuredLogMetadata,
    level: &Level,
    target: &str,
    message: String,
    mut promoted: BTreeMap<String, Value>,
    fields: BTreeMap<String, Value>,
) -> Map<String, Value> {
    let mut line = Map::new();
    line.insert(
        "timestamp".to_string(),
        Value::String(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
    );
    line.insert(
        "level".to_string(),
        Value::String(level.as_str().to_ascii_lowercase()),
    );
    line.insert("message".to_string(), Value::String(message));
    line.insert(
        "service".to_string(),
        Value::String(metadata.service.clone()),
    );
    line.insert(
        "environment".to_string(),
        optional_json_string(&metadata.environment),
    );
    line.insert(
        "deployment_id".to_string(),
        optional_json_string(&metadata.deployment_id),
    );
    line.insert(
        "replica_id".to_string(),
        optional_json_string(&metadata.replica_id),
    );
    line.insert(
        "git_sha".to_string(),
        optional_json_string(&metadata.git_sha),
    );
    line.insert("target".to_string(), Value::String(target.to_string()));

    for field in [
        "thread_id",
        "run_id",
        "turn_id",
        "tool_name",
        "request_id",
        "error_kind",
        "duration_ms",
        "tool_call_id",
    ] {
        line.insert(
            field.to_string(),
            promoted.remove(field).unwrap_or(Value::Null),
        );
    }

    let mut extra_fields = fields;
    merge_json_map(&mut extra_fields, promoted);
    if !extra_fields.is_empty() {
        line.insert(
            "fields".to_string(),
            Value::Object(extra_fields.into_iter().collect()),
        );
    }
    line
}

fn optional_json_string(value: &Option<String>) -> Value {
    value
        .as_ref()
        .map(|value| Value::String(value.clone()))
        .unwrap_or(Value::Null)
}

fn merge_json_map(target: &mut BTreeMap<String, Value>, source: BTreeMap<String, Value>) {
    for (key, value) in source {
        target.insert(key, value);
    }
}

fn json_value_to_log_string(value: Value) -> String {
    match value {
        Value::String(value) => value,
        value => value.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex};
    use tracing_subscriber::prelude::*;

    static CAPTURED_LOG_LINES: LazyLock<Mutex<Vec<String>>> =
        LazyLock::new(|| Mutex::new(Vec::new()));

    fn capture_log_line(line: &str) {
        CAPTURED_LOG_LINES
            .lock()
            .expect("capture log lock")
            .push(line.to_string());
    }

    #[test]
    fn structured_line_has_stable_fields() {
        let metadata = StructuredLogMetadata {
            service: "ironclaw".to_string(),
            environment: Some("production".to_string()),
            deployment_id: Some("dep_123".to_string()),
            replica_id: Some("replica_1".to_string()),
            git_sha: Some("abc123".to_string()),
        };
        let mut promoted = BTreeMap::new();
        promoted.insert(
            "thread_id".to_string(),
            Value::String("thread-a".to_string()),
        );
        promoted.insert("duration_ms".to_string(), Value::Number(42.into()));

        let line = structured_log_line(
            &metadata,
            &Level::INFO,
            "ironclaw",
            "started".to_string(),
            promoted,
            BTreeMap::new(),
        );

        assert_eq!(line["level"], "info");
        assert_eq!(line["service"], "ironclaw");
        assert_eq!(line["deployment_id"], "dep_123");
        assert_eq!(line["thread_id"], "thread-a");
        assert_eq!(line["duration_ms"], 42);
        assert!(line.contains_key("request_id"));
        assert!(line.contains_key("error_kind"));
    }

    #[test]
    fn structured_json_layer_emits_single_line_json_with_span_and_event_fields() {
        CAPTURED_LOG_LINES.lock().expect("capture log lock").clear();
        let subscriber = tracing_subscriber::registry().with(
            StructuredJsonLogLayer::with_line_writer("ironclaw-test", capture_log_line),
        );

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(
                "turn",
                thread_id = "thread-a",
                run_id = "run-a",
                duration_ms = "42",
                span_extra = "from-span",
            );
            let _guard = span.enter();
            tracing::info!(
                target: "ironclaw::test",
                turn_id = "turn-a",
                tool_name = "shell",
                request_id = "request-a",
                event_extra = "from-event",
                "tool completed"
            );
        });

        let lines = CAPTURED_LOG_LINES.lock().expect("capture log lock");
        assert_eq!(lines.len(), 1, "expected one captured log line: {lines:?}");
        assert!(
            !lines[0].contains('\n'),
            "structured logs must stay single-line"
        );
        let line: Value =
            serde_json::from_str(&lines[0]).expect("captured log line parses as JSON");

        assert_eq!(line["level"], "info");
        assert_eq!(line["message"], "tool completed");
        assert_eq!(line["service"], "ironclaw-test");
        assert_eq!(line["target"], "ironclaw::test");
        assert_eq!(line["thread_id"], "thread-a");
        assert_eq!(line["run_id"], "run-a");
        assert_eq!(line["turn_id"], "turn-a");
        assert_eq!(line["tool_name"], "shell");
        assert_eq!(line["request_id"], "request-a");
        assert_eq!(line["duration_ms"], 42);
        assert_eq!(line["fields"]["span_extra"], "from-span");
        assert_eq!(line["fields"]["event_extra"], "from-event");
    }
}
