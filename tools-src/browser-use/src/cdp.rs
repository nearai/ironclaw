use serde_json::{json, Value};

use crate::constants::CDP_TIMEOUT_MS;
use crate::near::agent::host as wit_host;

pub struct CdpClient {
    message_id: u32,
}

impl CdpClient {
    pub fn new() -> Self {
        Self { message_id: 0 }
    }

    pub fn next_id(&mut self) -> u32 {
        self.message_id += 1;
        self.message_id
    }

    pub fn connect(base_url: &str) -> Result<wit_host::WsConnection, String> {
        let ws_url = http_to_ws_url(base_url);
        wit_host::ws_connect(&ws_url)
    }

    /// Connect with connection pooling so the WebSocket (and Browserless
    /// browser context) persists across tool invocations.
    pub fn connect_pooled(
        base_url: &str,
        pool_key: &str,
    ) -> Result<wit_host::WsConnection, String> {
        let ws_url = http_to_ws_url(base_url);
        wit_host::ws_connect_pooled(&ws_url, pool_key)
    }

    pub fn send_command(
        &mut self,
        conn: &wit_host::WsConnection,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, String> {
        let id = self.next_id();
        let command = json!({
            "id": id,
            "method": method,
            "params": params.unwrap_or(json!({}))
        });

        conn.send(&command.to_string())?;

        // Loop to skip CDP events until we get our response (matching id).
        // Safety limit prevents infinite looping if the server floods events.
        const MAX_RECV_ATTEMPTS: u32 = 2000;
        for _ in 0..MAX_RECV_ATTEMPTS {
            let response_str = conn.recv(Some(CDP_TIMEOUT_MS))?;

            let response: Value = serde_json::from_str(&response_str)
                .map_err(|e| format!("Invalid CDP response: {e}"))?;

            // Skip event messages (they have "method" but no "id")
            if response.get("id").is_none() {
                continue;
            }

            if response.get("id").and_then(Value::as_u64) != Some(id as u64) {
                continue;
            }

            if let Some(error) = response.get("error") {
                return Err(format!("CDP error: {}", error));
            }

            return Ok(response);
        }
        Err(format!(
            "CDP response for id {} not received after {} messages",
            id, MAX_RECV_ATTEMPTS
        ))
    }

    pub fn create_browser_context(
        &mut self,
        conn: &wit_host::WsConnection,
    ) -> Result<String, String> {
        let response = self.send_command(conn, "Target.createBrowserContext", None)?;

        response
            .get("result")
            .and_then(|r| r.get("browserContextId"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Missing browserContextId in response".to_string())
    }

    pub fn dispose_browser_context(
        &mut self,
        conn: &wit_host::WsConnection,
        context_id: &str,
    ) -> Result<(), String> {
        self.send_command(
            conn,
            "Target.disposeBrowserContext",
            Some(json!({ "browserContextId": context_id })),
        )?;
        Ok(())
    }

    pub fn get_targets(&mut self, conn: &wit_host::WsConnection) -> Result<Vec<Value>, String> {
        let response = self.send_command(conn, "Target.getTargets", None)?;

        response
            .get("result")
            .and_then(|r| r.get("targetInfos"))
            .and_then(|v| v.as_array())
            .cloned()
            .ok_or_else(|| "Missing targetInfos in response".to_string())
    }
}

pub fn http_to_ws_url(http_url: &str) -> String {
    let url = http_url.trim_end_matches('/');

    let base = if url.starts_with("http://") {
        url.replace("http://", "ws://")
    } else if url.starts_with("https://") {
        url.replace("https://", "wss://")
    } else {
        format!("ws://{}", url)
    };

    let path = crate::constants::CDP_WS_PATH.trim_start_matches('/');
    if path.is_empty() {
        base
    } else {
        format!("{}/{}", base, path)
    }
}

pub fn generate_session_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let millis = wit_host::now_millis();
    // Combine timestamp + monotonic counter to avoid collisions
    format!("session-{}-{}", millis, seq)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_to_ws_url_http() {
        assert_eq!(
            http_to_ws_url("http://127.0.0.1:9222"),
            "ws://127.0.0.1:9222"
        );
    }

    #[test]
    fn test_http_to_ws_url_https() {
        assert_eq!(
            http_to_ws_url("https://localhost:9222"),
            "wss://localhost:9222"
        );
    }

    #[test]
    fn test_http_to_ws_url_strips_trailing_slash() {
        assert_eq!(
            http_to_ws_url("http://127.0.0.1:9222/"),
            "ws://127.0.0.1:9222"
        );
    }

    #[test]
    fn test_http_to_ws_url_bare_host() {
        assert_eq!(http_to_ws_url("127.0.0.1:9222"), "ws://127.0.0.1:9222");
    }

    #[test]
    fn test_http_to_ws_url_no_double_slash() {
        let result = http_to_ws_url("http://127.0.0.1:9222");
        assert!(
            !result.ends_with("//"),
            "should not end with double slash: {}",
            result
        );
    }

    #[test]
    fn test_cdp_client_increments_ids() {
        let mut client = CdpClient::new();
        assert_eq!(client.next_id(), 1);
        assert_eq!(client.next_id(), 2);
        assert_eq!(client.next_id(), 3);
    }
}
