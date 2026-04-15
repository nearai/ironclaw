//! Feishu/Lark Calendar WASM Tool for IronClaw.
//!
//! List, create, and delete calendar events using the Feishu Open API.
//!
//! # Authentication
//!
//! Store your Feishu tenant_access_token:
//! `ironclaw secret set feishu_access_token <token>`
//!
//! Get a token at: https://open.feishu.cn/

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const BASE_URL: &str = "https://open.feishu.cn";
const MAX_RETRIES: u32 = 3;

struct FeishuCalendarTool;

impl exports::near::agent::tool::Guest for FeishuCalendarTool {
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
        "Manage Feishu/Lark calendar events (飞书日历). \
         List, create, and delete events on a calendar. \
         Authentication is handled via the 'feishu_access_token' \
         secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    calendar_id: Option<String>,
    event_id: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    start_time: Option<TimeSpec>,
    end_time: Option<TimeSpec>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TimeSpec {
    timestamp: Option<String>,
    timezone: Option<String>,
}

// --- Feishu API response types ---

#[derive(Debug, Deserialize)]
struct FeishuResponse<T> {
    code: i32,
    msg: Option<String>,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct ListEventsData {
    #[serde(default)]
    items: Vec<EventItem>,
    has_more: Option<bool>,
    page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventItem {
    event_id: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    start_time: Option<EventTime>,
    end_time: Option<EventTime>,
    status: Option<String>,
    organizer_calendar_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventTime {
    timestamp: Option<String>,
    timezone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateEventData {
    event: Option<EventItem>,
}

#[derive(Debug, Deserialize)]
struct EmptyData {}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !near::agent::host::secret_exists("feishu_access_token") {
        return Err(
            "Feishu access token not found in secret store. Set it with: \
             ironclaw secret set feishu_access_token <token>. \
             Get a token at: https://open.feishu.cn/"
                .into(),
        );
    }

    match params.action.as_str() {
        "list_events" => list_events(&params),
        "create_event" => create_event(&params),
        "delete_event" => delete_event(&params),
        _ => Err(format!(
            "Unknown action '{}'. Expected: list_events, create_event, delete_event",
            params.action
        )),
    }
}

fn require_calendar_id(params: &Params) -> Result<&str, String> {
    let calendar_id = params
        .calendar_id
        .as_deref()
        .ok_or("'calendar_id' is required")?;
    if calendar_id.is_empty() {
        return Err("'calendar_id' must not be empty".into());
    }
    Ok(calendar_id)
}

fn list_events(params: &Params) -> Result<String, String> {
    let calendar_id = require_calendar_id(params)?;
    let page_size = params.page_size.unwrap_or(50).clamp(1, 500);

    let mut url = format!(
        "{BASE_URL}/open-apis/calendar/v4/calendars/{calendar_id}/events?page_size={page_size}"
    );
    if let Some(ref pt) = params.page_token {
        url.push_str(&format!("&page_token={pt}"));
    }

    let resp_body = feishu_request("GET", &url, None)?;
    let resp: FeishuResponse<ListEventsData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let data = resp.data.unwrap_or(ListEventsData {
        items: vec![],
        has_more: Some(false),
        page_token: None,
    });

    let events: Vec<serde_json::Value> = data
        .items
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "event_id": e.event_id,
                "summary": e.summary,
                "description": e.description,
                "start_time": e.start_time.map(|t| serde_json::json!({"timestamp": t.timestamp, "timezone": t.timezone})),
                "end_time": e.end_time.map(|t| serde_json::json!({"timestamp": t.timestamp, "timezone": t.timezone})),
                "status": e.status,
            })
        })
        .collect();

    let output = serde_json::json!({
        "action": "list_events",
        "calendar_id": calendar_id,
        "has_more": data.has_more.unwrap_or(false),
        "page_token": data.page_token,
        "event_count": events.len(),
        "events": events,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn create_event(params: &Params) -> Result<String, String> {
    let calendar_id = require_calendar_id(params)?;

    let summary = params
        .summary
        .as_deref()
        .ok_or("'summary' is required for create_event")?;
    if summary.is_empty() {
        return Err("'summary' must not be empty".into());
    }

    let start_time = params
        .start_time
        .as_ref()
        .ok_or("'start_time' is required for create_event")?;
    let end_time = params
        .end_time
        .as_ref()
        .ok_or("'end_time' is required for create_event")?;

    let start_ts = start_time
        .timestamp
        .as_deref()
        .ok_or("'start_time.timestamp' is required")?;
    let end_ts = end_time
        .timestamp
        .as_deref()
        .ok_or("'end_time.timestamp' is required")?;

    let url = format!(
        "{BASE_URL}/open-apis/calendar/v4/calendars/{calendar_id}/events"
    );

    let mut body = serde_json::json!({
        "summary": summary,
        "start_time": {
            "timestamp": start_ts,
        },
        "end_time": {
            "timestamp": end_ts,
        },
    });

    if let Some(ref desc) = params.description {
        body["description"] = serde_json::json!(desc);
    }
    if let Some(ref tz) = start_time.timezone {
        body["start_time"]["timezone"] = serde_json::json!(tz);
    }
    if let Some(ref tz) = end_time.timezone {
        body["end_time"]["timezone"] = serde_json::json!(tz);
    }

    let resp_body = feishu_request("POST", &url, Some(&body))?;
    let resp: FeishuResponse<CreateEventData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let event = resp.data.and_then(|d| d.event);
    let output = serde_json::json!({
        "action": "create_event",
        "calendar_id": calendar_id,
        "event_id": event.as_ref().and_then(|e| e.event_id.as_deref()),
        "summary": event.as_ref().and_then(|e| e.summary.as_deref()),
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn delete_event(params: &Params) -> Result<String, String> {
    let calendar_id = require_calendar_id(params)?;
    let event_id = params
        .event_id
        .as_deref()
        .ok_or("'event_id' is required for delete_event")?;

    if event_id.is_empty() {
        return Err("'event_id' must not be empty".into());
    }

    let url = format!(
        "{BASE_URL}/open-apis/calendar/v4/calendars/{calendar_id}/events/{event_id}"
    );

    let resp_body = feishu_request("DELETE", &url, None)?;
    let resp: FeishuResponse<EmptyData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let output = serde_json::json!({
        "action": "delete_event",
        "calendar_id": calendar_id,
        "event_id": event_id,
        "success": true,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn feishu_request(
    method: &str,
    url: &str,
    body: Option<&serde_json::Value>,
) -> Result<String, String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-Feishu-Tool/0.1"
    });

    let mut attempt = 0;
    loop {
        attempt += 1;

        let body_bytes = body.map(|b| b.to_string().into_bytes());
        let resp = near::agent::host::http_request(
            method,
            url,
            &headers.to_string(),
            body_bytes.as_deref(),
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
                    "Feishu API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body_str = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "Feishu API error (HTTP {}): {}",
            resp.status, body_str
        ));
    }
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "The action to perform: 'list_events' (列出日程), 'create_event' (创建日程), 'delete_event' (删除日程)",
            "enum": ["list_events", "create_event", "delete_event"]
        },
        "calendar_id": {
            "type": "string",
            "description": "Calendar ID (required for all actions)"
        },
        "event_id": {
            "type": "string",
            "description": "Event ID (required for delete_event)"
        },
        "summary": {
            "type": "string",
            "description": "Event title/summary (required for create_event)"
        },
        "description": {
            "type": "string",
            "description": "Event description (optional for create_event)"
        },
        "start_time": {
            "type": "object",
            "description": "Event start time (required for create_event)",
            "properties": {
                "timestamp": {
                    "type": "string",
                    "description": "Unix timestamp in seconds"
                },
                "timezone": {
                    "type": "string",
                    "description": "Timezone (e.g. 'Asia/Shanghai')"
                }
            },
            "required": ["timestamp"]
        },
        "end_time": {
            "type": "object",
            "description": "Event end time (required for create_event)",
            "properties": {
                "timestamp": {
                    "type": "string",
                    "description": "Unix timestamp in seconds"
                },
                "timezone": {
                    "type": "string",
                    "description": "Timezone (e.g. 'Asia/Shanghai')"
                }
            },
            "required": ["timestamp"]
        },
        "page_size": {
            "type": "integer",
            "description": "Number of events per page (1-500, default 50, for list_events)",
            "minimum": 1,
            "maximum": 500,
            "default": 50
        },
        "page_token": {
            "type": "string",
            "description": "Pagination token for next page (for list_events)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(FeishuCalendarTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_list_events_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "items": [
                    {
                        "event_id": "evt_abc123",
                        "summary": "团队周会",
                        "description": "每周例会",
                        "start_time": {"timestamp": "1700000000", "timezone": "Asia/Shanghai"},
                        "end_time": {"timestamp": "1700003600", "timezone": "Asia/Shanghai"},
                        "status": "confirmed"
                    }
                ],
                "has_more": false,
                "page_token": null
            }
        }"#;
        let resp: FeishuResponse<ListEventsData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert_eq!(data.items.len(), 1);
        assert_eq!(data.items[0].event_id.as_deref(), Some("evt_abc123"));
        assert_eq!(data.items[0].summary.as_deref(), Some("团队周会"));
        assert_eq!(data.items[0].status.as_deref(), Some("confirmed"));
        assert_eq!(data.has_more, Some(false));
    }

    #[test]
    fn test_parse_empty_events_response() {
        let json = r#"{"code": 0, "msg": "success", "data": {"items": [], "has_more": false}}"#;
        let resp: FeishuResponse<ListEventsData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert!(resp.data.unwrap().items.is_empty());
    }

    #[test]
    fn test_parse_create_event_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "event": {
                    "event_id": "evt_new001",
                    "summary": "新会议",
                    "start_time": {"timestamp": "1700010000"},
                    "end_time": {"timestamp": "1700013600"}
                }
            }
        }"#;
        let resp: FeishuResponse<CreateEventData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let event = resp.data.unwrap().event.unwrap();
        assert_eq!(event.event_id.as_deref(), Some("evt_new001"));
        assert_eq!(event.summary.as_deref(), Some("新会议"));
    }

    #[test]
    fn test_parse_delete_event_response() {
        let json = r#"{"code": 0, "msg": "success", "data": {}}"#;
        let resp: FeishuResponse<EmptyData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
    }

    #[test]
    fn test_parse_time_spec() {
        let json = r#"{"timestamp": "1700000000", "timezone": "Asia/Shanghai"}"#;
        let ts: TimeSpec = serde_json::from_str(json).unwrap();
        assert_eq!(ts.timestamp.as_deref(), Some("1700000000"));
        assert_eq!(ts.timezone.as_deref(), Some("Asia/Shanghai"));
    }

    #[test]
    fn test_parse_error_response() {
        let json = r#"{"code": 99991663, "msg": "token invalid", "data": null}"#;
        let resp: FeishuResponse<ListEventsData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 99991663);
        assert_eq!(resp.msg.as_deref(), Some("token invalid"));
    }
}
