//! DingTalk Calendar WASM Tool for IronClaw.
//!
//! Manages calendar events via the DingTalk Open API (钉钉日历).
//! Supports listing, creating, and deleting events on the primary calendar.
//!
//! # Authentication
//!
//! Store your DingTalk access token:
//! `ironclaw secret set dingtalk_access_token <token>`
//!
//! Get a token at: https://open.dingtalk.com/

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const API_BASE: &str = "https://api.dingtalk.com/v1.0";
const MAX_RETRIES: u32 = 3;

struct DingTalkCalendarTool;

impl exports::near::agent::tool::Guest for DingTalkCalendarTool {
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
        "Manage DingTalk calendar events (钉钉日历). List, create, update, and delete events on \
         the primary calendar. Authentication is handled via the 'dingtalk_access_token' \
         secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    #[serde(rename = "userId")]
    user_id: Option<String>,
    #[serde(rename = "eventId")]
    event_id: Option<String>,
    summary: Option<String>,
    #[serde(rename = "startTime")]
    start_time: Option<String>,
    #[serde(rename = "endTime")]
    end_time: Option<String>,
}

// --- Response types ---

#[derive(Debug, Deserialize)]
struct EventListResponse {
    events: Option<Vec<CalendarEvent>>,
    #[serde(rename = "nextToken")]
    next_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CalendarEvent {
    id: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    start: Option<EventTime>,
    end: Option<EventTime>,
    #[serde(rename = "isAllDay")]
    is_all_day: Option<bool>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventTime {
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
    #[serde(rename = "timeZone")]
    time_zone: Option<String>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("参数解析失败: {e}"))?;

    if !near::agent::host::secret_exists("dingtalk_access_token") {
        return Err(
            "未找到钉钉 access_token。请使用 ironclaw secret set dingtalk_access_token <token> 设置。\
             获取方式: https://open.dingtalk.com/"
                .into(),
        );
    }

    let user_id = params.user_id.as_deref();

    match params.action.as_str() {
        "list_events" => {
            let uid = user_id.ok_or_else(|| "list_events 操作需要 userId 参数".to_string())?;
            list_events(uid)
        }
        "create_event" => {
            let uid = user_id.ok_or_else(|| "create_event 操作需要 userId 参数".to_string())?;
            let summary = params
                .summary
                .ok_or_else(|| "create_event 操作需要 summary 参数".to_string())?;
            let start = params
                .start_time
                .ok_or_else(|| "create_event 操作需要 startTime 参数".to_string())?;
            let end = params
                .end_time
                .ok_or_else(|| "create_event 操作需要 endTime 参数".to_string())?;
            create_event(uid, &summary, &start, &end)
        }
        "update_event" => {
            let uid = user_id.ok_or_else(|| "update_event 操作需要 userId 参数".to_string())?;
            let event_id = params
                .event_id
                .as_deref()
                .ok_or_else(|| "update_event 操作需要 eventId 参数".to_string())?;
            update_event(uid, event_id, &params)
        }
        "delete_event" => {
            let uid = user_id.ok_or_else(|| "delete_event 操作需要 userId 参数".to_string())?;
            let event_id = params
                .event_id
                .ok_or_else(|| "delete_event 操作需要 eventId 参数".to_string())?;
            delete_event(uid, &event_id)
        }
        other => Err(format!(
            "未知操作: '{other}'。支持的操作: list_events, create_event, update_event, delete_event"
        )),
    }
}

fn list_events(user_id: &str) -> Result<String, String> {
    let url = format!(
        "{API_BASE}/calendar/users/{user_id}/calendars/primary/events?maxResults=50"
    );
    let body = do_request("GET", &url, None)?;
    let resp: EventListResponse =
        serde_json::from_str(&body).map_err(|e| format!("响应解析失败: {e}"))?;

    let events = resp.events.unwrap_or_default();
    let output = serde_json::json!({
        "action": "list_events",
        "count": events.len(),
        "events": events.iter().map(|e| {
            let mut ev = serde_json::json!({
                "id": e.id,
                "summary": e.summary,
            });
            if let Some(ref start) = e.start {
                ev["startTime"] = serde_json::json!(start.date_time);
            }
            if let Some(ref end) = e.end {
                ev["endTime"] = serde_json::json!(end.date_time);
            }
            if let Some(ref desc) = e.description {
                ev["description"] = serde_json::json!(desc);
            }
            if let Some(all_day) = e.is_all_day {
                ev["isAllDay"] = serde_json::json!(all_day);
            }
            if let Some(ref status) = e.status {
                ev["status"] = serde_json::json!(status);
            }
            ev
        }).collect::<Vec<_>>(),
        "nextToken": resp.next_token,
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn create_event(user_id: &str, summary: &str, start: &str, end: &str) -> Result<String, String> {
    let url = format!(
        "{API_BASE}/calendar/users/{user_id}/calendars/primary/events"
    );
    let req_body = serde_json::json!({
        "summary": summary,
        "start": { "dateTime": start },
        "end": { "dateTime": end },
    });
    let body_bytes = req_body.to_string().into_bytes();
    let body = do_request("POST", &url, Some(&body_bytes))?;
    let resp: CalendarEvent =
        serde_json::from_str(&body).map_err(|e| format!("响应解析失败: {e}"))?;

    let output = serde_json::json!({
        "action": "create_event",
        "event": {
            "id": resp.id,
            "summary": resp.summary,
        },
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn update_event(user_id: &str, event_id: &str, params: &Params) -> Result<String, String> {
    let url = format!(
        "{API_BASE}/calendar/users/{user_id}/calendars/primary/events/{event_id}"
    );

    let mut req_body = serde_json::json!({});
    if let Some(ref summary) = params.summary {
        req_body["summary"] = serde_json::json!(summary);
    }
    if let Some(ref start) = params.start_time {
        req_body["start"] = serde_json::json!({"dateTime": start});
    }
    if let Some(ref end) = params.end_time {
        req_body["end"] = serde_json::json!({"dateTime": end});
    }

    let body_bytes = req_body.to_string().into_bytes();
    let body = do_request("PUT", &url, Some(&body_bytes))?;
    let resp: CalendarEvent =
        serde_json::from_str(&body).map_err(|e| format!("响应解析失败: {e}"))?;

    let output = serde_json::json!({
        "action": "update_event",
        "event": {
            "id": resp.id,
            "summary": resp.summary,
        },
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn delete_event(user_id: &str, event_id: &str) -> Result<String, String> {
    let url = format!(
        "{API_BASE}/calendar/users/{user_id}/calendars/primary/events/{event_id}"
    );
    do_request("DELETE", &url, None)?;

    let output = serde_json::json!({
        "action": "delete_event",
        "eventId": event_id,
        "success": true,
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn do_request(method: &str, url: &str, body: Option<&[u8]>) -> Result<String, String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-DingTalkCalendar-Tool/0.1"
    });

    let response = {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let resp = near::agent::host::http_request(
                method,
                url,
                &headers.to_string(),
                body,
                None,
            )
            .map_err(|e| format!("HTTP 请求失败: {e}"))?;

            if resp.status >= 200 && resp.status < 300 {
                break resp;
            }

            if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
                near::agent::host::log(
                    near::agent::host::LogLevel::Warn,
                    &format!(
                        "DingTalk API 错误 {} (尝试 {}/{}), 重试中...",
                        resp.status, attempt, MAX_RETRIES
                    ),
                );
                continue;
            }

            let body_str = String::from_utf8_lossy(&resp.body);
            return Err(format!("DingTalk API 错误 (HTTP {}): {}", resp.status, body_str));
        }
    };

    String::from_utf8(response.body).map_err(|e| format!("响应编码错误: {e}"))
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "操作类型: list_events (列出日程), create_event (创建日程), update_event (更新日程), delete_event (删除日程)",
            "enum": ["list_events", "create_event", "update_event", "delete_event"]
        },
        "userId": {
            "type": "string",
            "description": "用户 ID (所有操作必填)"
        },
        "eventId": {
            "type": "string",
            "description": "日程事件 ID (delete_event 操作必填)"
        },
        "summary": {
            "type": "string",
            "description": "日程标题 (create_event 操作必填)"
        },
        "startTime": {
            "type": "string",
            "description": "开始时间，ISO 8601 格式，如 2025-01-01T09:00:00+08:00 (create_event 必填)"
        },
        "endTime": {
            "type": "string",
            "description": "结束时间，ISO 8601 格式，如 2025-01-01T10:00:00+08:00 (create_event 必填)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(DingTalkCalendarTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_event_list_response() {
        let json = r#"{
            "events": [
                {
                    "id": "evt_001",
                    "summary": "团队周会",
                    "start": { "dateTime": "2025-01-06T10:00:00+08:00", "timeZone": "Asia/Shanghai" },
                    "end": { "dateTime": "2025-01-06T11:00:00+08:00", "timeZone": "Asia/Shanghai" },
                    "isAllDay": false,
                    "status": "confirmed"
                }
            ],
            "nextToken": "page2"
        }"#;
        let resp: EventListResponse = serde_json::from_str(json).unwrap();
        let events = resp.events.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id.as_deref(), Some("evt_001"));
        assert_eq!(events[0].summary.as_deref(), Some("团队周会"));
        assert_eq!(events[0].is_all_day, Some(false));
        assert_eq!(
            events[0].start.as_ref().and_then(|t| t.date_time.as_deref()),
            Some("2025-01-06T10:00:00+08:00")
        );
    }

    #[test]
    fn test_parse_empty_events() {
        let json = r#"{"events": []}"#;
        let resp: EventListResponse = serde_json::from_str(json).unwrap();
        assert!(resp.events.unwrap().is_empty());
    }

    #[test]
    fn test_parse_calendar_event_response() {
        let json = r#"{
            "id": "evt_002",
            "summary": "新建日程"
        }"#;
        let resp: CalendarEvent = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id.as_deref(), Some("evt_002"));
        assert_eq!(resp.summary.as_deref(), Some("新建日程"));
        assert!(resp.start.is_none());
    }

    #[test]
    fn test_parse_update_event_response() {
        let json = r#"{
            "id": "evt_upd001",
            "summary": "更新后的日程",
            "start": { "dateTime": "2025-01-06T14:00:00+08:00" },
            "end": { "dateTime": "2025-01-06T15:00:00+08:00" }
        }"#;
        let resp: CalendarEvent = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id.as_deref(), Some("evt_upd001"));
        assert_eq!(resp.summary.as_deref(), Some("更新后的日程"));
        assert_eq!(
            resp.start.as_ref().and_then(|t| t.date_time.as_deref()),
            Some("2025-01-06T14:00:00+08:00")
        );
    }

    #[test]
    fn test_parse_event_with_description() {
        let json = r#"{
            "id": "evt_003",
            "summary": "项目评审",
            "description": "Q1 项目进度评审",
            "status": "tentative"
        }"#;
        let resp: CalendarEvent = serde_json::from_str(json).unwrap();
        assert_eq!(resp.description.as_deref(), Some("Q1 项目进度评审"));
        assert_eq!(resp.status.as_deref(), Some("tentative"));
    }
}
