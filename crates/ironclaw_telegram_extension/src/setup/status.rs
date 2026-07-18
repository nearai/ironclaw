use chrono::{DateTime, Utc};
use ironclaw_host_api::SecretHandle;
use ironclaw_product_adapters::AdapterInstallationId;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bot_api::TelegramBotApiError;

/// The route every deployment registers with Telegram (`setWebhook`). Pinned
/// to the unified-extension-runtime path so registrations survive the port.
pub const TELEGRAM_UPDATES_ROUTE_PATH: &str = "/webhooks/extensions/telegram/updates";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramInstallationSetup {
    pub bot_id: i64,
    pub bot_username: String,
    pub webhook_url: String,
    pub bot_token_handle: SecretHandle,
    pub webhook_secret_handle: SecretHandle,
    pub revision: u64,
    pub updated_at: DateTime<Utc>,
}

impl TelegramInstallationSetup {
    /// Installation identity is the bot: rotating the same bot's token keeps
    /// pairings; pointing at a different bot re-scopes them by design.
    pub fn installation_id(&self) -> Result<AdapterInstallationId, TelegramSetupError> {
        AdapterInstallationId::new(format!("tg-bot-{}", self.bot_id)).map_err(|error| {
            TelegramSetupError::InvalidField {
                field: "bot_id",
                reason: error.to_string(),
            }
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct TelegramInstallationSetupUpdate {
    /// New bot token; `None`/blank means "keep the existing token".
    pub bot_token: Option<SecretString>,
    /// Explicit public webhook URL override; `None` derives it from the
    /// deployment public base URL.
    pub webhook_url_override: Option<String>,
}

/// Redacted, serialize-only status projection for the admin UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TelegramInstallationSetupStatus {
    pub configured: bool,
    pub bot_username: Option<String>,
    pub bot_token_configured: bool,
    pub webhook_url: Option<String>,
    pub revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TelegramSetupError {
    #[error("invalid telegram setup field {field}: {reason}")]
    InvalidField { field: &'static str, reason: String },
    #[error("missing telegram setup field {field}")]
    MissingField { field: &'static str },
    #[error("telegram setup store unavailable")]
    StoreUnavailable,
    #[error("telegram setup changed concurrently; retry the operation")]
    ConcurrentUpdate,
    #[error("telegram secret store unavailable: {reason}")]
    SecretStoreUnavailable { reason: &'static str },
    #[error(
        "no public base URL is configured; set a webhook URL override or configure the deployment public origin"
    )]
    PublicUrlMissing,
    #[error("telegram bot api call failed: {reason}")]
    BotApi { reason: String },
}

impl From<TelegramBotApiError> for TelegramSetupError {
    fn from(error: TelegramBotApiError) -> Self {
        TelegramSetupError::BotApi {
            reason: error.to_string(),
        }
    }
}
