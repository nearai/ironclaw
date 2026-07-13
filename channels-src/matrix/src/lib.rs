//! Matrix channel skeleton for IronClaw Reborn.

wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusUpdate,
};
use near::agent::channel_host;
use serde::Deserialize;

const WEBHOOK_PATH: &str = "/webhook/matrix";
const MIN_POLL_INTERVAL_MS: u32 = 30_000;
const SUPPORTED_CALLBACKS: &[&str] = &[
    "on_start",
    "on_http_request",
    "on_poll",
    "on_status",
    "on_shutdown",
];

#[derive(Debug, Deserialize)]
struct MatrixConfig {
    #[allow(dead_code)]
    #[serde(default)]
    homeserver_url: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    user_id: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    polling_enabled: bool,
    #[serde(default = "default_poll_interval_ms")]
    poll_interval_ms: u32,
}

fn default_poll_interval_ms() -> u32 {
    MIN_POLL_INTERVAL_MS
}

struct MatrixChannel;

impl Guest for MatrixChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config: MatrixConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse Matrix config: {}", e))?;

        if config.polling_enabled && config.poll_interval_ms < MIN_POLL_INTERVAL_MS {
            return Err(format!(
                "poll_interval_ms must be at least {}",
                MIN_POLL_INTERVAL_MS
            ));
        }

        host_log(
            channel_host::LogLevel::Info,
            "Matrix channel skeleton starting",
        );

        Ok(ChannelConfig {
            display_name: "Matrix".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: WEBHOOK_PATH.to_string(),
                methods: vec!["POST".to_string()],
                require_secret: true,
            }],
            poll: if config.polling_enabled {
                Some(PollConfig {
                    interval_ms: config.poll_interval_ms,
                    enabled: true,
                })
            } else {
                None
            },
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        if req.path != WEBHOOK_PATH {
            return json_response(404, serde_json::json!({"error": "unknown Matrix endpoint"}));
        }

        if !req.method.eq_ignore_ascii_case("POST") {
            return json_response(405, serde_json::json!({"error": "method not allowed"}));
        }

        if !req.secret_validated {
            return json_response(401, serde_json::json!({"error": "webhook secret required"}));
        }

        json_response(
            501,
            serde_json::json!({"error": "Matrix webhook intake is not implemented"}),
        )
    }

    fn on_poll() {
        host_log(
            channel_host::LogLevel::Debug,
            "Matrix polling callback invoked before sync implementation",
        );
    }

    fn on_respond(_response: AgentResponse) -> Result<(), String> {
        Err(unsupported_callback_error(
            "on_respond",
            "Matrix outbound send is not implemented in the R001 skeleton",
        ))
    }

    fn on_status(_update: StatusUpdate) {}

    fn on_broadcast(_user_id: String, _response: AgentResponse) -> Result<(), String> {
        Err(unsupported_callback_error(
            "on_broadcast",
            "Matrix broadcast is not implemented in the R001 skeleton",
        ))
    }

    fn on_shutdown() {
        host_log(
            channel_host::LogLevel::Info,
            "Matrix channel skeleton shutting down",
        );
    }
}

#[cfg(not(test))]
fn host_log(level: channel_host::LogLevel, message: &str) {
    channel_host::log(level, message);
}

#[cfg(test)]
fn host_log(_level: channel_host::LogLevel, _message: &str) {}

fn json_response(status: u16, value: serde_json::Value) -> OutgoingHttpResponse {
    OutgoingHttpResponse {
        status,
        headers_json: serde_json::json!({"content-type": "application/json"}).to_string(),
        body: value.to_string().into_bytes(),
    }
}

fn unsupported_callback_error(callback: &str, reason: &str) -> String {
    format!(
        "{} unsupported: {}. Supported callbacks: {}",
        callback,
        reason,
        SUPPORTED_CALLBACKS.join(", ")
    )
}

export!(MatrixChannel);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn default_config() -> String {
        serde_json::json!({
            "homeserver_url": null,
            "user_id": null,
            "device_id": null,
            "polling_enabled": false
        })
        .to_string()
    }

    fn request(method: &str, path: &str) -> IncomingHttpRequest {
        IncomingHttpRequest {
            method: method.to_string(),
            path: path.to_string(),
            headers_json: "{}".to_string(),
            query_json: "{}".to_string(),
            body: Vec::new(),
            secret_validated: true,
        }
    }

    fn body_json(response: &OutgoingHttpResponse) -> Value {
        serde_json::from_slice(&response.body).expect("response body should be JSON")
    }

    #[test]
    fn on_start_registers_matrix_endpoint_without_default_polling() {
        let config = <MatrixChannel as Guest>::on_start(default_config()).expect("valid config");

        assert_eq!(config.display_name, "Matrix");
        assert_eq!(config.http_endpoints.len(), 1);
        assert_eq!(config.http_endpoints[0].path, "/webhook/matrix");
        assert_eq!(config.http_endpoints[0].methods, vec!["POST"]);
        assert!(config.http_endpoints[0].require_secret);
        assert!(config.poll.is_none());
    }

    #[test]
    fn on_start_enables_polling_when_configured() {
        let config_json = serde_json::json!({
            "polling_enabled": true,
            "poll_interval_ms": 45000
        })
        .to_string();

        let config = <MatrixChannel as Guest>::on_start(config_json).expect("valid config");
        let poll = config.poll.expect("polling should be configured");

        assert!(poll.enabled);
        assert_eq!(poll.interval_ms, 45_000);
    }

    #[test]
    fn on_start_rejects_invalid_config_json() {
        let err = <MatrixChannel as Guest>::on_start("{".to_string())
            .expect_err("invalid JSON must be rejected");
        assert!(err.contains("Failed to parse Matrix config"));
    }

    #[test]
    fn on_start_rejects_polling_below_wit_minimum() {
        let config_json = serde_json::json!({
            "polling_enabled": true,
            "poll_interval_ms": 1000
        })
        .to_string();

        let err = <MatrixChannel as Guest>::on_start(config_json)
            .expect_err("poll intervals below 30s must fail");
        assert!(err.contains("poll_interval_ms"));
    }

    #[test]
    fn http_request_rejects_unknown_path() {
        let response = <MatrixChannel as Guest>::on_http_request(request("POST", "/wrong"));

        assert_eq!(response.status, 404);
        assert_eq!(body_json(&response)["error"], "unknown Matrix endpoint");
    }

    #[test]
    fn http_request_rejects_non_post_method() {
        let response = <MatrixChannel as Guest>::on_http_request(request("GET", "/webhook/matrix"));

        assert_eq!(response.status, 405);
        assert_eq!(body_json(&response)["error"], "method not allowed");
    }

    #[test]
    fn http_request_rejects_missing_webhook_secret() {
        let mut req = request("POST", "/webhook/matrix");
        req.secret_validated = false;

        let response = <MatrixChannel as Guest>::on_http_request(req);

        assert_eq!(response.status, 401);
        assert_eq!(body_json(&response)["error"], "webhook secret required");
    }

    #[test]
    fn http_request_reports_skeleton_not_implemented() {
        let response =
            <MatrixChannel as Guest>::on_http_request(request("POST", "/webhook/matrix"));

        assert_eq!(response.status, 501);
        assert_eq!(
            body_json(&response)["error"],
            "Matrix webhook intake is not implemented"
        );
    }

    #[test]
    fn outbound_callbacks_return_explicit_skeleton_errors() {
        let response = AgentResponse {
            message_id: "m1".to_string(),
            content: "hello".to_string(),
            thread_id: None,
            metadata_json: "{}".to_string(),
            attachments: Vec::new(),
        };

        let respond_err = <MatrixChannel as Guest>::on_respond(response.clone())
            .expect_err("respond should not silently succeed");
        assert!(respond_err.contains("Matrix outbound send is not implemented"));

        let broadcast_err =
            <MatrixChannel as Guest>::on_broadcast("@alice:example.org".to_string(), response)
                .expect_err("broadcast should not silently succeed");
        assert!(broadcast_err.contains("Matrix broadcast is not implemented"));
    }

    #[test]
    fn supported_callbacks_are_declared_as_non_empty_array() {
        assert!(!SUPPORTED_CALLBACKS.is_empty());
        assert_eq!(SUPPORTED_CALLBACKS[0], "on_start");
        assert!(SUPPORTED_CALLBACKS.contains(&"on_http_request"));
        assert!(SUPPORTED_CALLBACKS.contains(&"on_poll"));
    }

    #[test]
    fn unsupported_callback_errors_include_callback_name_and_supported_callbacks() {
        let err = unsupported_callback_error("on_respond", "not implemented");

        assert!(err.contains("on_respond"));
        assert!(err.contains("not implemented"));
        assert!(err.contains("Supported callbacks"));
        assert!(err.contains("on_start"));
        assert!(err.contains("on_http_request"));
    }

    #[test]
    fn manifest_declares_host_managed_matrix_capabilities() {
        let manifest: Value = serde_json::from_str(include_str!("../matrix.capabilities.json"))
            .expect("manifest should parse as JSON");

        assert_eq!(manifest["type"], "channel");
        assert_eq!(manifest["name"], "matrix");
        assert_eq!(manifest["wit_version"], "0.3.1");
        assert_eq!(
            manifest["capabilities"]["http"]["credentials"]["matrix_token"]["secret_name"],
            "matrix_access_token"
        );
        assert_eq!(
            manifest["capabilities"]["channel"]["webhook"]["secret_name"],
            "matrix_webhook_secret"
        );
        assert_eq!(
            manifest["capabilities"]["channel"]["workspace_prefix"],
            "channels/matrix/"
        );
    }

    #[test]
    fn readme_documents_skeleton_non_goals() {
        let readme = include_str!("../README.md");

        assert!(readme.contains("Matrix channel skeleton"));
        assert!(readme.contains("does not implement live sync"));
        assert!(readme.contains("does not contain real credentials"));
        assert!(readme.contains("matrix_webhook_secret"));
        assert!(readme.contains("SSRF"));
    }
}
