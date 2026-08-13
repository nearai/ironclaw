use std::collections::HashMap;

use crate::types::{GitHubWebhookRequest, SystemEventIntent, ToolWebhookResponse};

fn header_value<'a>(headers: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    let lower = key.to_ascii_lowercase();
    headers
        .iter()
        .find(|(k, _)| k.to_ascii_lowercase() == lower)
        .map(|(_, v)| v.as_str())
}

pub(crate) fn handle_webhook(webhook: GitHubWebhookRequest) -> Result<String, String> {
    let event = header_value(&webhook.headers, "x-github-event")
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "Missing X-GitHub-Event header".to_string())?;

    let payload = webhook
        .body_json
        .ok_or_else(|| "Missing webhook.body_json".to_string())?;

    let event_type = github_event_type(event, &payload);
    let enriched_payload = github_enriched_payload(event, &webhook.headers, &payload, &event_type);

    let resp = ToolWebhookResponse {
        accepted: true,
        emit_events: vec![SystemEventIntent {
            source: "github".to_string(),
            event_type,
            payload: enriched_payload,
        }],
    };
    serde_json::to_string(&resp).map_err(|e| format!("Failed to encode webhook response: {e}"))
}

fn github_event_type(event: &str, payload: &serde_json::Value) -> String {
    let base = match event {
        "issues" => "issue",
        "pull_request" => "pr",
        "issue_comment" => {
            if payload.pointer("/issue/pull_request").is_some() {
                "pr.comment"
            } else {
                "issue.comment"
            }
        }
        "pull_request_review" => "pr.review",
        "pull_request_review_comment" => "pr.review_comment",
        "pull_request_review_thread" => "pr.review_thread",
        "check_suite" => "ci.check_suite",
        "check_run" => "ci.check_run",
        "status" => "ci.status",
        other => other,
    };

    if let Some(action) = payload.get("action").and_then(|v| v.as_str()) {
        if !action.is_empty() {
            return format!("{base}.{action}");
        }
    }

    base.to_string()
}

fn github_enriched_payload(
    raw_event: &str,
    headers: &HashMap<String, String>,
    payload: &serde_json::Value,
    event_type: &str,
) -> serde_json::Value {
    fn put_if_missing(
        obj: &mut serde_json::Map<String, serde_json::Value>,
        key: &str,
        val: Option<serde_json::Value>,
    ) {
        if !obj.contains_key(key) {
            if let Some(v) = val {
                obj.insert(key.to_string(), v);
            }
        }
    }

    let mut obj = payload
        .as_object()
        .cloned()
        .unwrap_or_else(serde_json::Map::new);

    put_if_missing(
        &mut obj,
        "event",
        Some(serde_json::Value::String(raw_event.to_string())),
    );
    put_if_missing(
        &mut obj,
        "event_type",
        Some(serde_json::Value::String(event_type.to_string())),
    );
    put_if_missing(
        &mut obj,
        "delivery_id",
        header_value(headers, "x-github-delivery")
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "action",
        payload
            .get("action")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "repository_name",
        payload
            .pointer("/repository/full_name")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "repository_owner",
        payload
            .pointer("/repository/owner/login")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "sender_login",
        payload
            .pointer("/sender/login")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "issue_number",
        payload.pointer("/issue/number").cloned(),
    );
    // For `issue_comment` webhooks on PRs, `/pull_request/number` is absent but
    // `/issue/number` is present and `/issue/pull_request` exists. Fall back to
    // `/issue/number` so PR-comment events carry `pr_number`.
    let pr_number = payload
        .pointer("/pull_request/number")
        .cloned()
        .or_else(|| {
            if payload.pointer("/issue/pull_request").is_some() {
                payload.pointer("/issue/number").cloned()
            } else {
                None
            }
        });
    put_if_missing(&mut obj, "pr_number", pr_number);
    put_if_missing(
        &mut obj,
        "comment_author",
        payload
            .pointer("/comment/user/login")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "comment_body",
        payload
            .pointer("/comment/body")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "review_state",
        payload
            .pointer("/review/state")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "pr_state",
        payload
            .pointer("/pull_request/state")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "pr_merged",
        payload.pointer("/pull_request/merged").cloned(),
    );
    put_if_missing(
        &mut obj,
        "pr_draft",
        payload.pointer("/pull_request/draft").cloned(),
    );
    put_if_missing(
        &mut obj,
        "base_branch",
        payload
            .pointer("/pull_request/base/ref")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "head_branch",
        payload
            .pointer("/pull_request/head/ref")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "ci_status",
        payload
            .pointer("/check_run/status")
            .or_else(|| payload.pointer("/check_suite/status"))
            .or_else(|| payload.pointer("/status"))
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "ci_conclusion",
        payload
            .pointer("/check_run/conclusion")
            .or_else(|| payload.pointer("/check_suite/conclusion"))
            .or_else(|| payload.pointer("/state"))
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );

    serde_json::Value::Object(obj)
}
