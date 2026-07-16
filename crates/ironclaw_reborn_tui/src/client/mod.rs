//! API client for `ironclaw-reborn serve`'s WebChat v2 surface
//! (`/api/webchat/v2/*`). Per-domain method groups live in sibling files
//! (`threads`, `gates`, `automations`, `llm`, `session`, `events`); this
//! module owns only [`ApiClient`], [`ClientError`], and the shared
//! `send`/`send_json`/`send_unit` request helpers.

use std::fmt;

use serde::de::DeserializeOwned;
use thiserror::Error;

pub mod automations;
pub mod events;
pub mod gates;
pub mod llm;
pub mod runs;
pub mod session;
pub mod threads;

pub use automations::AutomationSummary;
pub use session::SessionInfo;
pub use threads::{ThreadMessageSummary, ThreadSummary, TimelinePage};

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("unauthorized")]
    Unauthorized,
    #[error("not found")]
    NotFound,
    #[error("rate limited")]
    RateLimited,
    // Keep the response body available to callers that explicitly route it
    // through redacted diagnostics, but never include it in Display: Display
    // is rendered directly in the TUI status bar.
    #[error("server error {status}")]
    Server { status: u16, body: String },
    #[error("invalid response: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("reconnect budget exhausted ({attempts} attempts in {window_secs}s)")]
    ReconnectBudgetExhausted { attempts: u8, window_secs: u64 },
    #[error("SSE stream parse error: {0}")]
    StreamParse(String),
    #[error("SSE stream protocol error: {0}")]
    StreamProtocol(&'static str),
}

pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the bearer token.
        f.debug_struct("ApiClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl ApiClient {
    pub fn new(base_url: String, token: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            token,
        }
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    /// Sends `builder` with the bearer token attached and classifies the
    /// response status once, shared by every other `send_*` helper (and by
    /// `client/events.rs`'s SSE connect step) so status-code mapping lives
    /// in exactly one place.
    async fn send(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, ClientError> {
        let response = builder.bearer_auth(&self.token).send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ClientError::Unauthorized);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ClientError::NotFound);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ClientError::RateLimited);
        }
        if !status.is_success() {
            let body = response.text().await?;
            return Err(ClientError::Server {
                status: status.as_u16(),
                body,
            });
        }
        Ok(response)
    }

    pub(crate) async fn send_json<T: DeserializeOwned>(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<T, ClientError> {
        let response = self.send(builder).await?;
        let body = response.text().await?;
        serde_json::from_str(&body).map_err(ClientError::Decode)
    }

    pub(crate) async fn send_unit(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<(), ClientError> {
        self.send(builder).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{ApiClient, ClientError};

    async fn spawn_truncated_error_server() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind truncated error server");
        let addr = listener.local_addr().expect("truncated error server addr");
        tokio::spawn(async move {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let Ok(read) = socket.read(&mut buffer).await else {
                    return;
                };
                if read == 0 {
                    return;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = socket
                .write_all(
                    b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 64\r\nconnection: close\r\n\r\npartial body",
                )
                .await;
        });
        addr
    }

    #[test]
    fn server_error_display_does_not_expose_response_body() {
        let error = ClientError::Server {
            status: 500,
            body: "secret backend detail".to_string(),
        };

        assert_eq!(error.to_string(), "server error 500");
        assert!(!error.to_string().contains("secret backend detail"));
    }

    #[tokio::test]
    async fn truncated_error_body_propagates_transport_failure() {
        let addr = spawn_truncated_error_server().await;
        let client = ApiClient::new(format!("http://{addr}"), "test-token".to_string());

        let error = client
            .send(client.http.get(client.url("/truncated")))
            .await
            .expect_err("truncated response body must not become a server error");

        assert!(matches!(error, ClientError::Transport(_)));
    }
}
