//! Session bootstrap, used as the `serve` health check.
//!
//! Wire source: `WebUiV2SessionResponse` (`ironclaw_webui_v2::handlers`).
//! The client only needs `tenant_id`/`user_id`; extra fields
//! (`capabilities`, `features`, `attachments`) are ignored by default serde
//! behavior (no `deny_unknown_fields`).

use super::{ApiClient, ClientError};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SessionInfo {
    pub tenant_id: String,
    pub user_id: String,
}

impl ApiClient {
    pub async fn session(&self) -> Result<SessionInfo, ClientError> {
        self.send_json(self.http.get(self.url("/api/webchat/v2/session")))
            .await
    }
}
