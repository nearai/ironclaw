//! Linear webhook ingress tool.
//!
//! Receives Linear webhook events at `POST /webhook/tools/linear` and emits
//! system events that routines can react to:
//!
//! - `linear.issue.create` / `linear.issue.update` / `linear.issue.remove`
//! - `linear.comment.create` / `linear.comment.update` / `linear.comment.remove`
//!
//! Auth: HMAC-SHA256 via `Linear-Signature` header. Store the signing secret
//! as `linear_webhook_secret` in the secrets store.

use std::time::Instant;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput};
use crate::tools::wasm::WebhookCapability;

pub struct LinearWebhookTool;

#[async_trait]
impl Tool for LinearWebhookTool {
    fn name(&self) -> &str {
        "linear"
    }

    fn description(&self) -> &str {
        "Receives Linear webhook events (issue updates, comments) and emits system events \
         that routines can react to for commitment sync and triggered agent work."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    fn webhook_capability(&self) -> Option<WebhookCapability> {
        Some(WebhookCapability {
            // Secret stored under this name in the secrets store.
            hmac_secret_name: Some("linear_webhook_secret".to_string()),
            // Linear sends `Linear-Signature: <raw-hex>` — no prefix.
            hmac_signature_header: Some("linear-signature".to_string()),
            hmac_prefix: Some("".to_string()),
            ..Default::default()
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let body = params
            .get("webhook")
            .and_then(|w| w.get("body_json"))
            .ok_or_else(|| {
                ToolError::InvalidParameters(
                    "linear tool is webhook-only; invoke via POST /webhook/tools/linear"
                        .to_string(),
                )
            })?;

        let events = extract_events(body);

        Ok(ToolOutput::success(
            serde_json::json!({ "emit_events": events }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Parse a Linear webhook payload into system event intents.
///
/// Linear webhook shape:
/// ```json
/// { "type": "Issue"|"Comment"|..., "action": "create"|"update"|"remove",
///   "data": { ... }, "updatedFrom": { ... } }
/// ```
fn extract_events(body: &serde_json::Value) -> Vec<serde_json::Value> {
    let event_type = body.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("");

    match (event_type, action) {
        ("Issue", action) => issue_events(body, action),
        ("Comment", action) => comment_events(body, action),
        _ => vec![],
    }
}

fn issue_events(body: &serde_json::Value, action: &str) -> Vec<serde_json::Value> {
    let Some(data) = body.get("data") else {
        return vec![];
    };
    // title is user-controlled free text — excluded to prevent prompt injection.
    // The routine can fetch it from Linear if needed.
    vec![serde_json::json!({
        "source": "linear",
        "event_type": format!("linear.issue.{action}"),
        "payload": {
            "id": data.get("id"),
            "identifier": data.get("identifier"),
            "url": data.get("url"),
            "state": data.get("state"),
            "priority": data.get("priority"),
            "assignee": data.get("assignee"),
            "updated_from": body.get("updatedFrom"),
        }
    })]
}

fn comment_events(body: &serde_json::Value, action: &str) -> Vec<serde_json::Value> {
    let Some(data) = body.get("data") else {
        return vec![];
    };
    // body and issue.title are user-controlled free text — excluded to prevent
    // prompt injection. The routine fetches comment content via Linear API.
    let issue = data.get("issue").map(|i| {
        serde_json::json!({
            "id": i.get("id"),
            "identifier": i.get("identifier"),
            "url": i.get("url"),
        })
    });
    // user_email lifted to top level so routines can filter on it without an
    // LLM call (e.g. only fire for comments from the instance owner).
    let user_email = data
        .get("user")
        .and_then(|u| u.get("email"))
        .and_then(|v| v.as_str())
        .map(String::from);
    vec![serde_json::json!({
        "source": "linear",
        "event_type": format!("linear.comment.{action}"),
        "payload": {
            "id": data.get("id"),
            "issue_id": data.get("issueId"),
            "issue": issue,
            "user": data.get("user"),
            "user_email": user_email,
        }
    })]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> JobContext {
        JobContext::with_user("test", "webhook", "test")
    }

    #[test]
    fn webhook_capability_uses_linear_hmac_config() {
        let cap = LinearWebhookTool.webhook_capability().unwrap();
        assert_eq!(
            cap.hmac_secret_name.as_deref(),
            Some("linear_webhook_secret")
        );
        assert_eq!(
            cap.hmac_signature_header.as_deref(),
            Some("linear-signature")
        );
        assert_eq!(cap.hmac_prefix.as_deref(), Some(""));
        assert!(cap.secret_name.is_none());
        assert!(cap.signature_key_secret_name.is_none());
    }

    #[tokio::test]
    async fn execute_without_webhook_context_returns_error() {
        let err = LinearWebhookTool
            .execute(serde_json::json!({}), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn issue_update_emits_event() {
        let payload = serde_json::json!({
            "webhook": {
                "body_json": {
                    "type": "Issue",
                    "action": "update",
                    "data": {
                        "id": "issue-uuid",
                        "identifier": "ENG-42",
                        "title": "Fix the bug",
                        "url": "https://linear.app/team/issue/ENG-42",
                        "state": { "name": "Done", "type": "completed" },
                        "priority": 2,
                        "assignee": { "name": "Tobias" }
                    },
                    "updatedFrom": { "stateId": "old-state-uuid" }
                }
            }
        });

        let out = LinearWebhookTool.execute(payload, &ctx()).await.unwrap();
        let events = out.result["emit_events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        assert_eq!(ev["source"], "linear");
        assert_eq!(ev["event_type"], "linear.issue.update");
        assert_eq!(ev["payload"]["identifier"], "ENG-42");
        assert_eq!(ev["payload"]["state"]["type"], "completed");
        // title is user-controlled free text — must not appear in payload
        assert!(ev["payload"]["title"].is_null());
    }

    #[tokio::test]
    async fn comment_create_emits_event() {
        let payload = serde_json::json!({
            "webhook": {
                "body_json": {
                    "type": "Comment",
                    "action": "create",
                    "data": {
                        "id": "comment-uuid",
                        "body": "@tobias please research X",
                        "issueId": "issue-uuid",
                        "issue": {
                            "id": "issue-uuid",
                            "identifier": "ENG-42",
                            "title": "Research task",
                            "url": "https://linear.app/team/issue/ENG-42"
                        },
                        "user": {
                            "name": "Alice",
                            "email": "alice@example.com"
                        }
                    }
                }
            }
        });

        let out = LinearWebhookTool.execute(payload, &ctx()).await.unwrap();
        let events = out.result["emit_events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        assert_eq!(ev["event_type"], "linear.comment.create");
        assert_eq!(ev["payload"]["issue"]["identifier"], "ENG-42");
        // body and issue.title are user-controlled free text — must not appear in payload
        assert!(ev["payload"]["body"].is_null());
        assert!(ev["payload"]["issue"]["title"].is_null());
        // id and issue_id are still present for routing
        assert_eq!(ev["payload"]["id"], "comment-uuid");
        assert_eq!(ev["payload"]["issue_id"], "issue-uuid");
        // user_email is lifted to top level for pre-LLM payload filtering
        assert_eq!(ev["payload"]["user_email"], "alice@example.com");
        // user_email is null when user has no email in the webhook
        let payload_no_email = serde_json::json!({
            "webhook": {
                "body_json": {
                    "type": "Comment",
                    "action": "create",
                    "data": {
                        "id": "comment-uuid",
                        "issueId": "issue-uuid",
                        "issue": { "id": "issue-uuid", "identifier": "ENG-42", "url": "" },
                        "user": { "name": "Bot" }
                    }
                }
            }
        });
        let out2 = LinearWebhookTool.execute(payload_no_email, &ctx()).await.unwrap();
        let ev2 = &out2.result["emit_events"][0];
        assert!(ev2["payload"]["user_email"].is_null());
    }

    #[tokio::test]
    async fn unknown_event_type_emits_nothing() {
        let payload = serde_json::json!({
            "webhook": {
                "body_json": {
                    "type": "Project",
                    "action": "update",
                    "data": { "id": "proj-uuid" }
                }
            }
        });

        let out = LinearWebhookTool.execute(payload, &ctx()).await.unwrap();
        assert_eq!(out.result["emit_events"].as_array().unwrap().len(), 0);
    }

}
