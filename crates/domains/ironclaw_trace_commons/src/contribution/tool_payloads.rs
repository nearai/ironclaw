//! Per-tool sensitive-field redaction: the payload profiles, their rule
//! tables, and the value sanitizers they apply.

use std::path::PathBuf;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::*;

pub(crate) fn redact_tool_specific_payload(
    tool_name: Option<&str>,
    value: &Value,
    report: &mut RedactionReport,
) -> Value {
    let Some(profile) = tool_name.and_then(tool_payload_profile) else {
        return value.clone();
    };
    redact_tool_specific_value(value, profile, None, report)
}

pub(crate) fn redact_tool_specific_value(
    value: &Value,
    profile: ToolPayloadProfile,
    field_name: Option<&str>,
    report: &mut RedactionReport,
) -> Value {
    if let Some(action) = field_name.and_then(|field| tool_redaction_action(profile, field)) {
        report.increment("tool_sensitive_field");
        report.increment(format!("tool_sensitive_field:{}", action.label()));
        report.add_pii_label(action.label());
        return apply_tool_redaction_action(value, action);
    }

    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child)| {
                    (
                        key.clone(),
                        redact_tool_specific_value(child, profile, Some(key), report),
                    )
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|child| redact_tool_specific_value(child, profile, None, report))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ToolPayloadProfile {
    Browser,
    Calendar,
    Database,
    Email,
    Filesystem,
    IssueTracker,
    Messaging,
}

pub(crate) fn tool_payload_profile(tool_name: &str) -> Option<ToolPayloadProfile> {
    let lower = tool_name.to_ascii_lowercase();
    if lower.contains("email") || lower.contains("gmail") {
        Some(ToolPayloadProfile::Email)
    } else if lower.contains("calendar") {
        Some(ToolPayloadProfile::Calendar)
    } else if lower.contains("slack")
        || lower.contains("telegram")
        || lower.contains("signal")
        || lower.contains("discord")
    {
        Some(ToolPayloadProfile::Messaging)
    } else if lower.contains("github")
        || lower.contains("gitlab")
        || lower.contains("linear")
        || lower.contains("issue")
        || lower.contains("pull_request")
        || lower.contains("pr_")
    {
        Some(ToolPayloadProfile::IssueTracker)
    } else if lower.contains("browser")
        || lower.contains("http")
        || lower.contains("fetch")
        || lower.contains("url")
        || lower.contains("web")
    {
        Some(ToolPayloadProfile::Browser)
    } else if lower.contains("sql")
        || lower.contains("db")
        || lower.contains("database")
        || lower.contains("postgres")
        || lower.contains("libsql")
        || lower.contains("mysql")
    {
        Some(ToolPayloadProfile::Database)
    } else if lower.contains("file")
        || lower.contains("fs")
        || lower.contains("workspace")
        || lower.contains("shell")
        || lower.contains("exec")
    {
        Some(ToolPayloadProfile::Filesystem)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ToolRedactionAction {
    Replace(&'static str),
    SanitizeUrl(&'static str),
    RedactObjectValues(&'static str),
    SummarizeCollection(&'static str),
}

impl ToolRedactionAction {
    fn label(self) -> &'static str {
        match self {
            ToolRedactionAction::Replace(label)
            | ToolRedactionAction::SanitizeUrl(label)
            | ToolRedactionAction::RedactObjectValues(label)
            | ToolRedactionAction::SummarizeCollection(label) => label,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolSensitiveFieldRule {
    matcher: ToolFieldMatcher,
    action: ToolRedactionAction,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ToolFieldMatcher {
    Exact(&'static [&'static str]),
    Contains(&'static [&'static str]),
}

pub(crate) const EMAIL_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "to",
            "cc",
            "bcc",
            "from",
            "reply_to",
            "replyto",
            "recipient",
            "recipients",
            "sender",
        ]),
        action: ToolRedactionAction::SummarizeCollection("email_participant"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "subject", "body", "text", "html", "snippet", "message", "raw", "mime",
        ]),
        action: ToolRedactionAction::Replace("email_content"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["headers", "header"]),
        action: ToolRedactionAction::RedactObjectValues("email_header"),
    },
];

pub(crate) const CALENDAR_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Contains(&["attendee", "participant", "organizer", "creator"]),
        action: ToolRedactionAction::SummarizeCollection("calendar_participant"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "summary",
            "title",
            "description",
            "location",
            "notes",
            "calendar_id",
            "hangout_link",
            "conference_data",
            "conference_uri",
        ]),
        action: ToolRedactionAction::Replace("calendar_content"),
    },
];

pub(crate) const MESSAGING_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Contains(&[
            "channel",
            "conversation",
            "user",
            "member",
            "team",
            "workspace",
            "chat",
        ]),
        action: ToolRedactionAction::Replace("message_identity"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "text",
            "message",
            "body",
            "blocks",
            "attachments",
            "permalink",
            "thread",
            "thread_ts",
        ]),
        action: ToolRedactionAction::Replace("message_content"),
    },
];

pub(crate) const ISSUE_TRACKER_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "title",
            "body",
            "description",
            "comment",
            "comments",
            "summary",
            "content",
        ]),
        action: ToolRedactionAction::Replace("issue_content"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["url", "html_url", "api_url", "web_url", "href"]),
        action: ToolRedactionAction::SanitizeUrl("private_url"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Contains(&[
            "author",
            "assignee",
            "reviewer",
            "requester",
            "creator",
            "owner",
            "repo",
            "repository",
            "org",
            "organization",
            "project",
            "team",
            "user",
        ]),
        action: ToolRedactionAction::Replace("issue_identity"),
    },
];

pub(crate) const BROWSER_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["url", "href", "referrer", "referer", "current_url"]),
        action: ToolRedactionAction::SanitizeUrl("private_url"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["headers", "header", "cookies", "cookie"]),
        action: ToolRedactionAction::RedactObjectValues("browser_header"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["body", "html", "text", "title", "content", "dom"]),
        action: ToolRedactionAction::Replace("browser_content"),
    },
];

pub(crate) const DATABASE_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "query",
            "sql",
            "statement",
            "prepared_statement",
            "connection_string",
            "database_url",
        ]),
        action: ToolRedactionAction::Replace("database_content"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "params",
            "parameters",
            "args",
            "arguments",
            "values",
            "binds",
            "bindings",
            "query_params",
        ]),
        action: ToolRedactionAction::SummarizeCollection("database_query_param"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "row", "rows", "record", "records", "result", "results", "data",
        ]),
        action: ToolRedactionAction::SummarizeCollection("database_row"),
    },
];

pub(crate) const FILESYSTEM_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Contains(&["path", "file", "filename", "cwd", "directory"]),
        action: ToolRedactionAction::Replace("local_path"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "content", "contents", "command", "stdout", "stderr", "diff", "patch",
        ]),
        action: ToolRedactionAction::Replace("workspace_content"),
    },
];

pub(crate) fn tool_redaction_action(
    profile: ToolPayloadProfile,
    field_name: &str,
) -> Option<ToolRedactionAction> {
    let lower = field_name.to_ascii_lowercase();

    profile_rules(profile)
        .iter()
        .find(|rule| field_matches(&lower, rule.matcher))
        .map(|rule| rule.action)
}

pub(crate) fn profile_rules(profile: ToolPayloadProfile) -> &'static [ToolSensitiveFieldRule] {
    match profile {
        ToolPayloadProfile::Email => EMAIL_RULES,
        ToolPayloadProfile::Calendar => CALENDAR_RULES,
        ToolPayloadProfile::Messaging => MESSAGING_RULES,
        ToolPayloadProfile::IssueTracker => ISSUE_TRACKER_RULES,
        ToolPayloadProfile::Browser => BROWSER_RULES,
        ToolPayloadProfile::Database => DATABASE_RULES,
        ToolPayloadProfile::Filesystem => FILESYSTEM_RULES,
    }
}

pub(crate) fn field_matches(lower_field_name: &str, matcher: ToolFieldMatcher) -> bool {
    match matcher {
        ToolFieldMatcher::Exact(names) => names.contains(&lower_field_name),
        ToolFieldMatcher::Contains(fragments) => fragments
            .iter()
            .any(|fragment| lower_field_name.contains(fragment)),
    }
}

pub(crate) fn apply_tool_redaction_action(value: &Value, action: ToolRedactionAction) -> Value {
    match action {
        ToolRedactionAction::Replace(label) => redacted_scalar_or_summary(label, value),
        ToolRedactionAction::SanitizeUrl(label) => sanitize_url_value(value, label),
        ToolRedactionAction::RedactObjectValues(label) => redact_object_values(value, label),
        ToolRedactionAction::SummarizeCollection(label) => summarize_collection(label, value),
    }
}

pub(crate) fn redacted_scalar_or_summary(label: &str, value: &Value) -> Value {
    match value {
        Value::Array(_) | Value::Object(_) => summarize_collection(label, value),
        _ => Value::String(redacted_marker(label)),
    }
}

pub(crate) fn redact_object_values(value: &Value, label: &str) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.keys()
                .map(|key| (key.clone(), Value::String(redacted_marker(label))))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_object_values(item, label))
                .collect(),
        ),
        _ => Value::String(redacted_marker(label)),
    }
}

pub(crate) fn summarize_collection(label: &str, value: &Value) -> Value {
    match value {
        Value::Array(items) => serde_json::json!({
            "redacted": redacted_marker(label),
            "count": items.len(),
        }),
        Value::Object(map) => serde_json::json!({
            "redacted": redacted_marker(label),
            "field_count": map.len(),
        }),
        _ => Value::String(redacted_marker(label)),
    }
}

pub(crate) fn sanitize_url_value(value: &Value, label: &str) -> Value {
    match value {
        Value::String(url) => Value::String(sanitize_private_url(url, label)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| sanitize_url_value(item, label))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child)| (key.clone(), sanitize_url_value(child, label)))
                .collect(),
        ),
        _ => Value::String(redacted_marker(label)),
    }
}

pub(crate) fn sanitize_private_url(raw_url: &str, label: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(raw_url) else {
        return redacted_marker(label);
    };

    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return redacted_marker(label);
    }

    let has_private_components =
        url.path() != "/" || url.query().is_some() || url.fragment().is_some();
    if has_private_components {
        url.set_path("/[REDACTED_PATH]");
    }
    url.set_query(None);
    url.set_fragment(None);
    if !url.username().is_empty() {
        let _ = url.set_username("");
    }
    let _ = url.set_password(None);
    url.to_string()
}

pub(crate) fn redacted_marker(label: &str) -> String {
    format!("[REDACTED:{label}]")
}

pub(crate) fn count_sensitive_field_redactions(
    before: &Value,
    after: &Value,
    report: &mut RedactionReport,
) {
    match (before, after) {
        (Value::Object(before_map), Value::Object(after_map)) => {
            for (key, before_value) in before_map {
                if let Some(after_value) = after_map.get(key) {
                    count_sensitive_field_redactions(before_value, after_value, report);
                }
            }
        }
        (Value::Array(before_items), Value::Array(after_items)) => {
            for (before_value, after_value) in before_items.iter().zip(after_items.iter()) {
                count_sensitive_field_redactions(before_value, after_value, report);
            }
        }
        (before_value, Value::String(redacted))
            if redacted == "[REDACTED]" && before_value != after =>
        {
            report.increment("sensitive_field");
        }
        _ => {}
    }
}

pub(crate) fn apply_redaction_ranges(input: &str, ranges: &[std::ops::Range<usize>]) -> String {
    apply_labeled_ranges(input, ranges, "[REDACTED]")
}

pub(crate) fn apply_placeholder_regex(
    input: &str,
    regex: &Regex,
    label: &str,
    state: &mut RedactionState,
    report: &mut RedactionReport,
) -> String {
    let mut result = String::with_capacity(input.len());
    let mut last_end = 0usize;
    let mut changed = false;

    for mat in regex.find_iter(input) {
        let candidate = mat.as_str();
        if candidate.contains("<PRIVATE_") || candidate.contains("[REDACTED") {
            continue;
        }
        result.push_str(&input[last_end..mat.start()]);
        let placeholder = state.placeholders.placeholder_for(label, candidate);
        result.push_str(&placeholder);
        last_end = mat.end();
        report.increment(label);
        report.add_pii_label(label);
        changed = true;
    }

    if !changed {
        return input.to_string();
    }
    result.push_str(&input[last_end..]);
    result
}

pub(crate) fn apply_labeled_ranges(
    input: &str,
    ranges: &[std::ops::Range<usize>],
    replacement: &str,
) -> String {
    if ranges.is_empty() {
        return input.to_string();
    }

    let mut ranges = ranges.to_vec();
    ranges.sort_by_key(|range| range.start);

    let mut result = String::with_capacity(input.len());
    let mut last_end = 0;
    for range in ranges {
        if range.start < last_end {
            continue;
        }
        result.push_str(&input[last_end..range.start]);
        result.push_str(replacement);
        last_end = range.end;
    }
    result.push_str(&input[last_end..]);
    result
}

pub(crate) fn private_email_regex() -> &'static Regex {
    static PRIVATE_EMAIL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b")
            .expect("hardcoded private email regex must compile") // safety: hardcoded regex is covered by unit tests and should always compile.
    });
    &PRIVATE_EMAIL_REGEX
}

pub(crate) fn local_path_regex() -> &'static Regex {
    static LOCAL_PATH_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?x)(?:/Users|/home|/private/var|/tmp)/[^\s'"`<>{}\[\]]+"#)
            .expect("hardcoded local path regex must compile") // safety: hardcoded regex is covered by unit tests and should always compile.
    });
    &LOCAL_PATH_REGEX
}

pub(crate) fn trace_queue_secret_like_reason_regex() -> &'static Regex {
    static TRACE_QUEUE_SECRET_LIKE_REASON_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?ix)\b(?:sk|pk|rk|ghp|gho|ghu|glpat|xox[baprs])[-_a-z0-9]{8,}\b")
            .expect("hardcoded trace queue secret-like reason regex must compile") // safety: hardcoded regex is covered by queue diagnostics tests.
    });
    &TRACE_QUEUE_SECRET_LIKE_REASON_REGEX
}

pub(crate) fn remote_credit_explanation_url_regex() -> &'static Regex {
    static REMOTE_CREDIT_EXPLANATION_URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)\bhttps?://[^\s'"`<>{}\[\]]+"#)
            .expect("hardcoded remote credit explanation URL regex must compile") // safety: hardcoded regex is covered by local status-history safety tests.
    });
    &REMOTE_CREDIT_EXPLANATION_URL_REGEX
}

pub(crate) fn remote_credit_explanation_tenant_ref_regex() -> &'static Regex {
    static REMOTE_CREDIT_EXPLANATION_TENANT_REF_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\btenant[-_][a-z0-9][a-z0-9_-]{1,}\b")
            .expect("hardcoded remote credit explanation tenant-ref regex must compile") // safety: hardcoded regex is covered by local status-history safety tests.
    });
    &REMOTE_CREDIT_EXPLANATION_TENANT_REF_REGEX
}

pub(crate) fn placeholder_label_fragment(label: &str) -> String {
    let raw = label
        .strip_prefix("private_")
        .unwrap_or(label)
        .to_ascii_uppercase();
    raw.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

pub(crate) fn path_to_string(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}
