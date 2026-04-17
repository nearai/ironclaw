//! `crew_progress` — agent-authored "I'm at phase X, here's what I'm doing" beacon.
//!
//! Posts to `POST /api/internal/crew-rooms/:id/progress` on the LobsterPool
//! platform. Rendered server-side as a `progress` content_type so the frontend
//! timeline highlights it in blue with the phase tag as a pill.
//!
//! Auth: `lp_crew_a2a_token` injected by the host as `Authorization: Bearer`.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::{Deserialize, Serialize};

const PLATFORM_HOST: &str = "http://lobsterpool:3000";
const MAX_SUMMARY_LEN: usize = 8 * 1024;
const MAX_PHASE_LEN: usize = 64;
const MAX_RETRIES: u32 = 3;

struct CrewProgressTool;

impl exports::near::agent::tool::Guest for CrewProgressTool {
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
        "Report structured progress into a LobsterPool Crew Room. Use a short stable \
         phase tag ('investigating', 'drafting', 'blocked', 'done', ...) plus a one-line \
         summary so teammates can follow along without re-reading the whole timeline. \
         Prefer report_blocker if you actually need help; use this for visible momentum."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    room_id: String,
    phase: String,
    summary: String,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct RequestBody<'a> {
    phase: &'a str,
    summary: &'a str,
    #[serde(rename = "idempotencyKey", skip_serializing_if = "Option::is_none")]
    idempotency_key: Option<&'a str>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !is_valid_uuid(&params.room_id) {
        return Err("'room_id' must be a UUID".into());
    }
    if params.phase.trim().is_empty() {
        return Err("'phase' must not be empty".into());
    }
    if params.phase.len() > MAX_PHASE_LEN {
        return Err(format!(
            "'phase' exceeds {MAX_PHASE_LEN} bytes — keep it short (e.g. 'investigating')"
        ));
    }
    if params.summary.trim().is_empty() {
        return Err("'summary' must not be empty".into());
    }
    if params.summary.len() > MAX_SUMMARY_LEN {
        return Err(format!(
            "'summary' exceeds {MAX_SUMMARY_LEN} bytes (you sent {})",
            params.summary.len()
        ));
    }
    if let Some(key) = &params.idempotency_key {
        if !is_valid_uuid(key) {
            return Err("'idempotency_key' must be a UUID if provided".into());
        }
    }

    let body = RequestBody {
        phase: &params.phase,
        summary: &params.summary,
        idempotency_key: params.idempotency_key.as_deref(),
    };
    let body_json =
        serde_json::to_string(&body).map_err(|e| format!("Failed to build request: {e}"))?;

    let url = format!(
        "{PLATFORM_HOST}/api/internal/crew-rooms/{}/progress",
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
                    "crew progress got HTTP {} (attempt {}/{}); retrying",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "LobsterPool rejected progress (HTTP {}): {}",
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
            "description": "UUID of the Crew Room the agent is reporting progress in.",
            "pattern": "^[0-9a-fA-F-]{36}$"
        },
        "phase": {
            "type": "string",
            "description": "Short stable tag for the current phase of work. Examples: 'investigating', 'drafting', 'blocked', 'awaiting_review', 'done'. Keep it canonical — the frontend colour-codes by matching against known phase strings.",
            "minLength": 1,
            "maxLength": 64
        },
        "summary": {
            "type": "string",
            "description": "One-line human-readable summary of what you just did / are doing.",
            "minLength": 1,
            "maxLength": 8192
        },
        "idempotency_key": {
            "type": "string",
            "description": "Optional UUID key. Retrying with the same key collapses to the first progress row.",
            "pattern": "^[0-9a-fA-F-]{36}$"
        }
    },
    "required": ["room_id", "phase", "summary"],
    "additionalProperties": false
}"#;

export!(CrewProgressTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_uuid() {
        let p = r#"{"room_id": "nope", "phase": "x", "summary": "y"}"#;
        assert!(execute_inner(p).unwrap_err().contains("room_id"));
    }

    #[test]
    fn rejects_empty_phase() {
        let p = r#"{"room_id": "550e8400-e29b-41d4-a716-446655440000", "phase": "", "summary": "y"}"#;
        assert!(execute_inner(p).unwrap_err().contains("phase"));
    }

    #[test]
    fn rejects_long_phase() {
        let phase = "a".repeat(65);
        let p = format!(
            r#"{{"room_id": "550e8400-e29b-41d4-a716-446655440000", "phase": "{phase}", "summary": "y"}}"#
        );
        assert!(execute_inner(&p).unwrap_err().contains("phase"));
    }
}
