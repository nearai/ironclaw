//! `crew_blocker` — agent-authored "I'm stuck, need help from X" beacon.
//!
//! Posts to `POST /api/internal/crew-rooms/:id/blocker` on the LobsterPool
//! platform. Rendered server-side as a `blocker` content_type so the frontend
//! timeline highlights it in red and the notification writer pings every human
//! in `need_from`.
//!
//! Auth: `lp_crew_a2a_token` injected by the host as `Authorization: Bearer`.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::{Deserialize, Serialize};

const PLATFORM_HOST: &str = "http://lobsterpool:3000";
const MAX_REASON_LEN: usize = 8 * 1024;
const MAX_RETRIES: u32 = 3;

struct CrewBlockerTool;

impl exports::near::agent::tool::Guest for CrewBlockerTool {
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
        "Report a blocker inside a LobsterPool Crew Room. Use this when you cannot \
         continue without human or peer-agent intervention. The 'need_from' list should \
         name the teammates whose input can unblock you. The room timeline renders a red \
         highlight; mentioned humans get a notification."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    room_id: String,
    reason: String,
    #[serde(default)]
    need_from: Vec<MentionTarget>,
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
    reason: &'a str,
    #[serde(rename = "needFrom")]
    need_from: &'a [MentionTarget],
    #[serde(rename = "idempotencyKey", skip_serializing_if = "Option::is_none")]
    idempotency_key: Option<&'a str>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !is_valid_uuid(&params.room_id) {
        return Err("'room_id' must be a UUID".into());
    }
    if params.reason.trim().is_empty() {
        return Err("'reason' must not be empty".into());
    }
    if params.reason.len() > MAX_REASON_LEN {
        return Err(format!(
            "'reason' exceeds {MAX_REASON_LEN} bytes (you sent {})",
            params.reason.len()
        ));
    }
    if let Some(key) = &params.idempotency_key {
        if !is_valid_uuid(key) {
            return Err("'idempotency_key' must be a UUID if provided".into());
        }
    }

    let body = RequestBody {
        reason: &params.reason,
        need_from: &params.need_from,
        idempotency_key: params.idempotency_key.as_deref(),
    };
    let body_json =
        serde_json::to_string(&body).map_err(|e| format!("Failed to build request: {e}"))?;

    let url = format!(
        "{PLATFORM_HOST}/api/internal/crew-rooms/{}/blocker",
        params.room_id
    );
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json"
    })
    .to_string();

    post_with_retries(&url, &headers, &body_json)
}

fn post_with_retries(url: &str, headers: &str, body: &str) -> Result<String, String> {
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
                    "crew blocker got HTTP {} (attempt {}/{}); retrying",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "LobsterPool rejected blocker (HTTP {}): {}",
            resp.status, body
        ));
    }
}

fn is_valid_uuid(s: &str) -> bool {
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
            "description": "UUID of the Crew Room the agent is reporting a blocker in.",
            "pattern": "^[0-9a-fA-F-]{36}$"
        },
        "reason": {
            "type": "string",
            "description": "Plain-text reason the agent is stuck. Be specific about what you tried and where you're blocked.",
            "minLength": 1,
            "maxLength": 8192
        },
        "need_from": {
            "type": "array",
            "description": "Who can unblock the agent. Each entry is `{kind: 'agent'|'user', id: <uuid>}`. An empty list posts the blocker without @-ing anyone (less likely to get resolved — prefer naming at least one teammate).",
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
            "description": "Optional UUID key. Retrying with the same key collapses to the first blocker row.",
            "pattern": "^[0-9a-fA-F-]{36}$"
        }
    },
    "required": ["room_id", "reason"],
    "additionalProperties": false
}"#;

export!(CrewBlockerTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_uuid() {
        let p = r#"{"room_id": "nope", "reason": "stuck"}"#;
        assert!(execute_inner(p).unwrap_err().contains("room_id"));
    }

    #[test]
    fn rejects_empty_reason() {
        let p = r#"{"room_id": "550e8400-e29b-41d4-a716-446655440000", "reason": ""}"#;
        assert!(execute_inner(p).unwrap_err().contains("reason"));
    }

    #[test]
    fn valid_uuid_check() {
        assert!(is_valid_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!is_valid_uuid("550e8400-e29b-41d4-a716-44665544000z"));
    }
}
