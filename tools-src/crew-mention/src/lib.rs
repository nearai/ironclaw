//! `crew_mention` — agent-authored @-mention posted into a LobsterPool Crew Room.
//!
//! The agent's LLM calls this tool when it wants to address a specific teammate
//! (virtual shrimp or human) with a line of text. The tool shells out to the
//! platform's internal endpoint `POST /api/internal/crew-rooms/:id/mention`.
//!
//! Auth: the `lp_crew_a2a_token` secret — a short-TTL A2A envelope JWT — is
//! injected by the host as `Authorization: Bearer <token>`. The WASM guest never
//! sees the token.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::{Deserialize, Serialize};

const PLATFORM_HOST: &str = "http://lobsterpool:3000";
const MAX_CONTENT_LEN: usize = 8 * 1024;
const MAX_RETRIES: u32 = 3;

struct CrewMentionTool;

impl exports::near::agent::tool::Guest for CrewMentionTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        SCHEMA.to_string()
    }

    fn description() -> String {
        "Post an @-mention into a LobsterPool Crew Room the agent is a member of. \
         Use this when you need to get the attention of a specific teammate — \
         either another agent (pass kind=\"agent\") or a human (kind=\"user\"). \
         The target must be an active member of the same room."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    room_id: String,
    content: String,
    #[serde(default)]
    mention_targets: Vec<MentionTarget>,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
enum MentionTarget {
    Agent(String),
    User(String),
}

#[derive(Debug, Serialize)]
struct RequestBody<'a> {
    content: &'a str,
    #[serde(rename = "mentionTargets")]
    mention_targets: &'a [MentionTarget],
    #[serde(rename = "idempotencyKey", skip_serializing_if = "Option::is_none")]
    idempotency_key: Option<&'a str>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !is_valid_uuid(&params.room_id) {
        return Err("'room_id' must be a UUID".into());
    }
    if params.content.trim().is_empty() {
        return Err("'content' must not be empty".into());
    }
    if params.content.len() > MAX_CONTENT_LEN {
        return Err(format!(
            "'content' exceeds {MAX_CONTENT_LEN} bytes (you sent {})",
            params.content.len()
        ));
    }
    if let Some(key) = &params.idempotency_key {
        if !is_valid_uuid(key) {
            return Err("'idempotency_key' must be a UUID if provided".into());
        }
    }

    let body = RequestBody {
        content: &params.content,
        mention_targets: &params.mention_targets,
        idempotency_key: params.idempotency_key.as_deref(),
    };
    let body_json =
        serde_json::to_string(&body).map_err(|e| format!("Failed to build request: {e}"))?;

    let url = format!(
        "{PLATFORM_HOST}/api/internal/crew-rooms/{}/mention",
        params.room_id
    );
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json"
    })
    .to_string();

    post_with_retries(&url, &headers, &body_json, "mention")
}

/// Shared POST-with-retries helper; reused across the three crew tools.
pub(crate) fn post_with_retries(
    url: &str,
    headers: &str,
    body: &str,
    tool_label: &str,
) -> Result<String, String> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let resp = near::agent::host::http_request(
            "POST",
            url,
            headers,
            Some(body.as_bytes().to_vec()).as_deref(),
            None,
        )
        .map_err(|e| format!("HTTP request failed: {e}"))?;

        if resp.status >= 200 && resp.status < 300 {
            return String::from_utf8(resp.body)
                .map_err(|e| format!("Invalid UTF-8 response: {e}"));
        }

        if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
            near::agent::host::log(
                near::agent::host::LogLevel::Warn,
                &format!(
                    "crew {tool_label} got HTTP {} (attempt {}/{}); retrying",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "LobsterPool rejected crew {tool_label} (HTTP {}): {}",
            resp.status, body
        ));
    }
}

/// Minimal UUID v4 shape check without pulling in a UUID crate.
pub(crate) fn is_valid_uuid(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (i, b) in bytes.iter().enumerate() {
        let is_dash = matches!(i, 8 | 13 | 18 | 23);
        if is_dash {
            if *b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "room_id": {
            "type": "string",
            "description": "UUID of the Crew Room to post into. The calling agent must be an active member.",
            "pattern": "^[0-9a-fA-F-]{36}$"
        },
        "content": {
            "type": "string",
            "description": "The message text (Markdown subset, max 8 KiB). Sanitized server-side; HTML is stripped.",
            "minLength": 1,
            "maxLength": 8192
        },
        "mention_targets": {
            "type": "array",
            "description": "Structured @-mention list. Each target is `{kind: 'agent'|'user', id: <uuid>}`. The server also parses `@[[user:<uuid>]]` / `@[[agent:<uuid>]]` sentinels inline in content, so this field is optional when content already carries them.",
            "items": {
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["agent", "user"]
                    },
                    "id": {
                        "type": "string",
                        "pattern": "^[0-9a-fA-F-]{36}$"
                    }
                },
                "required": ["kind", "id"]
            }
        },
        "idempotency_key": {
            "type": "string",
            "description": "Optional UUID key. Retrying with the same key collapses to the first message; content is not overwritten.",
            "pattern": "^[0-9a-fA-F-]{36}$"
        }
    },
    "required": ["room_id", "content"],
    "additionalProperties": false
}"#;

export!(CrewMentionTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_uuid() {
        let p = r#"{"room_id": "not-a-uuid", "content": "hi"}"#;
        let err = execute_inner(p).unwrap_err();
        assert!(err.contains("room_id"));
    }

    #[test]
    fn rejects_empty_content() {
        let p = r#"{"room_id": "550e8400-e29b-41d4-a716-446655440000", "content": "   "}"#;
        let err = execute_inner(p).unwrap_err();
        assert!(err.contains("content"));
    }

    #[test]
    fn rejects_oversize_content() {
        let huge = "a".repeat(8193);
        let p = format!(
            r#"{{"room_id": "550e8400-e29b-41d4-a716-446655440000", "content": "{huge}"}}"#
        );
        let err = execute_inner(&p).unwrap_err();
        assert!(err.contains("8192"));
    }

    #[test]
    fn valid_uuid_check() {
        assert!(is_valid_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!is_valid_uuid("550e8400e29b41d4a716446655440000")); // no dashes
        assert!(!is_valid_uuid("550e8400-e29b-41d4-a716-44665544000g")); // bad hex
    }
}
