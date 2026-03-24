wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use std::collections::BTreeMap;

use m01_core::{
    IntelligenceLocalStatsParams, M01Action, M01Connection, M01Request, MailFacetParams,
    MailListParams, MailRelatedEmailsDetectionParams, MailTimeType, QueueKind, QueueStatsParams,
};
use serde::Deserialize;
use serde_json::{json, Value};

const API_KEY_SECRET_NAME: &str = "m01_api_key";
const DEFAULT_TIMEOUT_MS: u32 = 30_000;

struct M01Tool;

#[derive(Debug, Deserialize)]
struct ToolInput {
    base_url: String,
    #[serde(flatten)]
    action: ToolAction,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ToolAction {
    InvestigationMailList {
        start_time: String,
        end_time: String,
        #[serde(default = "default_page")]
        page: u32,
        #[serde(default = "default_page_size")]
        page_size: u32,
        #[serde(default = "default_sort_by")]
        sort_by: String,
        #[serde(default = "default_sort_order")]
        sort_order: String,
        #[serde(default)]
        time_type: ToolMailTimeType,
        query_builder: Option<String>,
        created_by: Option<String>,
        #[serde(default)]
        store_search_history: bool,
    },
    InvestigationMailTags {
        start_time: String,
        end_time: String,
        #[serde(default)]
        time_type: ToolMailTimeType,
    },
    InvestigationMailFileTypes {
        start_time: String,
        end_time: String,
        #[serde(default)]
        time_type: ToolMailTimeType,
    },
    InvestigationMailRelatedEmailsDetection {
        start_time: String,
        end_time: String,
        #[serde(default = "default_page")]
        page: u32,
        #[serde(default = "default_page_size")]
        page_size: u32,
        #[serde(default)]
        time_type: ToolMailTimeType,
        technique_name: String,
    },
    WorkflowQueueStats {
        queue: ToolQueueKind,
        start_time: Option<String>,
        end_time: Option<String>,
    },
    IntelligenceLocalStats {
        #[serde(default)]
        query: BTreeMap<String, String>,
    },
    SafeAdminFirewallZoneList,
    SystemAdminMonitorStatus,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum ToolMailTimeType {
    #[default]
    Timestamp,
    DetectionTimestamp,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ToolQueueKind {
    Active,
    Deep,
    Deferred,
    Quarantine,
}

impl exports::near::agent::tool::Guest for M01Tool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(output) => exports::near::agent::tool::Response {
                output: Some(output),
                error: None,
            },
            Err(error) => exports::near::agent::tool::Response {
                output: None,
                error: Some(error),
            },
        }
    }

    fn schema() -> String {
        json!({
            "type": "object",
            "required": ["base_url", "action"],
            "properties": {
                "base_url": {
                    "type": "string",
                    "description": "Full M01 base URL, such as https://m01.example.com. The host must match the installed tool allowlist."
                },
                "action": {
                    "type": "string",
                    "enum": [
                        "investigation_mail_list",
                        "investigation_mail_tags",
                        "investigation_mail_file_types",
                        "investigation_mail_related_emails_detection",
                        "workflow_queue_stats",
                        "intelligence_local_stats",
                        "safe_admin_firewall_zone_list",
                        "system_admin_monitor_status"
                    ]
                },
                "start_time": { "type": "string" },
                "end_time": { "type": "string" },
                "page": { "type": "integer", "minimum": 1, "default": 1 },
                "page_size": { "type": "integer", "minimum": 1, "default": 20 },
                "sort_by": { "type": "string", "default": "timestamp" },
                "sort_order": { "type": "string", "default": "desc" },
                "time_type": {
                    "type": "string",
                    "enum": ["timestamp", "detection_timestamp"],
                    "default": "timestamp"
                },
                "query_builder": { "type": "string" },
                "created_by": { "type": "string" },
                "store_search_history": { "type": "boolean", "default": false },
                "technique_name": { "type": "string" },
                "queue": {
                    "type": "string",
                    "enum": ["active", "deep", "deferred", "quarantine"]
                },
                "query": {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                }
            },
            "additionalProperties": false
        })
        .to_string()
    }

    fn description() -> String {
        "M01 read-only JSON tool for IronClaw. Reuses the shared m01-core request builders \
         from m01-cli and executes approved operations through IronClaw's HTTP sandbox with \
         host-injected x-api-key authentication."
            .to_string()
    }
}

export!(M01Tool);

fn execute_inner(params: &str) -> Result<String, String> {
    if !near::agent::host::secret_exists(API_KEY_SECRET_NAME) {
        return Err(
            "M01 API key not configured. Run `ironclaw tool auth m01` or set M01_API_KEY for IronClaw."
                .to_string(),
        );
    }

    let (base_url, request) = build_request_from_params(params)?;
    near::agent::host::log(
        near::agent::host::LogLevel::Info,
        &format!(
            "Executing M01 request: {} {}",
            request.method.as_str(),
            request.path
        ),
    );
    execute_request(&base_url, &request)
}

fn build_request_from_params(params: &str) -> Result<(String, M01Request), String> {
    let input: ToolInput =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;
    let action = input.action.into_core_action();
    Ok((input.base_url, action.build_request()))
}

fn execute_request(base_url: &str, request: &M01Request) -> Result<String, String> {
    let url = M01Connection::new(base_url.to_string())
        .url_for(request)
        .map_err(|e| format!("Invalid base_url or request: {e}"))?;

    let headers = match &request.content_type {
        Some(content_type) => json!({ "Content-Type": content_type }).to_string(),
        None => "{}".to_string(),
    };
    let body = match &request.json_body {
        Some(value) => {
            Some(serde_json::to_vec(value).map_err(|e| format!("Invalid JSON body: {e}"))?)
        }
        None => None,
    };

    let response = near::agent::host::http_request(
        request.method.as_str(),
        &url,
        &headers,
        body.as_deref(),
        Some(DEFAULT_TIMEOUT_MS),
    )?;

    if !(200..=299).contains(&response.status) {
        return Err(format_http_error(response.status, &response.body));
    }

    if response.body.is_empty() {
        return Ok(json!({ "status": response.status }).to_string());
    }

    let body_text = String::from_utf8(response.body)
        .map_err(|e| format!("Response was not valid UTF-8: {e}"))?;
    if let Ok(payload) = serde_json::from_str::<Value>(&body_text) {
        ensure_business_success(&payload)?;
        return serde_json::to_string_pretty(&payload)
            .map_err(|e| format!("Failed to format JSON response: {e}"));
    }

    Ok(body_text)
}

fn format_http_error(status: u16, body: &[u8]) -> String {
    let body_text = String::from_utf8_lossy(body).to_string();
    if let Ok(payload) = serde_json::from_str::<Value>(&body_text) {
        if let Some(message) = extract_error_message(&payload) {
            return message;
        }
    }

    format!("HTTP {status}: {}", body_text.trim())
}

fn ensure_business_success(payload: &Value) -> Result<(), String> {
    let code = payload
        .get("code")
        .and_then(Value::as_i64)
        .or_else(|| {
            payload
                .get("code")
                .and_then(Value::as_u64)
                .map(|v| v as i64)
        })
        .or_else(|| {
            payload
                .get("code")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
        });

    match code {
        Some(0 | 200 | 1000) | None => Ok(()),
        Some(_) => {
            Err(extract_error_message(payload).unwrap_or_else(|| "M01 business error".to_string()))
        }
    }
}

fn extract_error_message(payload: &Value) -> Option<String> {
    [
        payload.get("message").and_then(Value::as_str),
        payload.get("msg").and_then(Value::as_str),
        payload.get("error").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .find(|value| !value.trim().is_empty())
    .map(ToOwned::to_owned)
}

impl ToolAction {
    fn into_core_action(self) -> M01Action {
        match self {
            ToolAction::InvestigationMailList {
                start_time,
                end_time,
                page,
                page_size,
                sort_by,
                sort_order,
                time_type,
                query_builder,
                created_by,
                store_search_history,
            } => M01Action::InvestigationMailList(MailListParams {
                start_time,
                end_time,
                page,
                page_size,
                sort_by,
                sort_order,
                time_type: time_type.into(),
                query_builder,
                created_by,
                store_search_history,
            }),
            ToolAction::InvestigationMailTags {
                start_time,
                end_time,
                time_type,
            } => M01Action::InvestigationMailTags(MailFacetParams {
                start_time,
                end_time,
                time_type: time_type.into(),
            }),
            ToolAction::InvestigationMailFileTypes {
                start_time,
                end_time,
                time_type,
            } => M01Action::InvestigationMailFileTypes(MailFacetParams {
                start_time,
                end_time,
                time_type: time_type.into(),
            }),
            ToolAction::InvestigationMailRelatedEmailsDetection {
                start_time,
                end_time,
                page,
                page_size,
                time_type,
                technique_name,
            } => M01Action::InvestigationMailRelatedEmailsDetection(
                MailRelatedEmailsDetectionParams {
                    start_time,
                    end_time,
                    page,
                    page_size,
                    time_type: time_type.into(),
                    technique_name,
                },
            ),
            ToolAction::WorkflowQueueStats {
                queue,
                start_time,
                end_time,
            } => M01Action::WorkflowQueueStats(QueueStatsParams {
                kind: queue.into(),
                start_time,
                end_time,
            }),
            ToolAction::IntelligenceLocalStats { query } => {
                M01Action::IntelligenceLocalStats(IntelligenceLocalStatsParams {
                    query: query.into_iter().collect(),
                })
            }
            ToolAction::SafeAdminFirewallZoneList => M01Action::SafeAdminFirewallZoneList,
            ToolAction::SystemAdminMonitorStatus => M01Action::SystemAdminMonitorStatus,
        }
    }
}

impl From<ToolMailTimeType> for MailTimeType {
    fn from(value: ToolMailTimeType) -> Self {
        match value {
            ToolMailTimeType::Timestamp => MailTimeType::Timestamp,
            ToolMailTimeType::DetectionTimestamp => MailTimeType::DetectionTimestamp,
        }
    }
}

impl From<ToolQueueKind> for QueueKind {
    fn from(value: ToolQueueKind) -> Self {
        match value {
            ToolQueueKind::Active => QueueKind::Active,
            ToolQueueKind::Deep => QueueKind::Deep,
            ToolQueueKind::Deferred => QueueKind::Deferred,
            ToolQueueKind::Quarantine => QueueKind::Quarantine,
        }
    }
}

fn default_page() -> u32 {
    1
}

fn default_page_size() -> u32 {
    20
}

fn default_sort_by() -> String {
    "timestamp".to_string()
}

fn default_sort_order() -> String {
    "desc".to_string()
}

#[cfg(test)]
mod tests {
    use super::build_request_from_params;

    #[test]
    fn builds_request_for_mail_list_action() {
        let (base_url, request) = build_request_from_params(
            r#"{
                "base_url": "https://m01.example.com",
                "action": "investigation_mail_list",
                "start_time": "2026-03-20 00:00:00",
                "end_time": "2026-03-20 23:59:59"
            }"#,
        )
        .unwrap();

        assert_eq!(base_url, "https://m01.example.com");
        assert_eq!(request.method.as_str(), "POST");
        assert_eq!(request.path, "/mail/list");
        assert!(request.json_body.is_some());
    }

    #[test]
    fn builds_request_for_queue_stats_action() {
        let (_, request) = build_request_from_params(
            r#"{
                "base_url": "https://m01.example.com",
                "action": "workflow_queue_stats",
                "queue": "deferred",
                "start_time": "2026-03-20 00:00:00",
                "end_time": "2026-03-20 23:59:59"
            }"#,
        )
        .unwrap();

        assert_eq!(request.method.as_str(), "GET");
        assert_eq!(request.path, "/api/mail/deferred/statistics");
        assert_eq!(request.query.len(), 2);
    }
}
