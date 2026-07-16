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
    #[error("server error {status}: {body}")]
    Server { status: u16, body: String },
    #[error("invalid response: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("reconnect budget exhausted ({attempts} attempts in {window_secs}s)")]
    ReconnectBudgetExhausted { attempts: u8, window_secs: u64 },
    #[error("SSE stream parse error: {0}")]
    StreamParse(String),
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
            let body = response.text().await.unwrap_or_default();
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
