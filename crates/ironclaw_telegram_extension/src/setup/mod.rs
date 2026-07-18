//! Durable Telegram installation setup and secret boundary.
//!
//! One bot per deployment, operator-managed at runtime. This module owns the
//! only place WebUI-submitted Telegram secrets are written to the shared
//! `SecretStore` and the only place runtime code resolves those handles back
//! to material. The save pipeline is fail-closed: token validation (`getMe`)
//! and webhook registration (`setWebhook`) both succeed before anything is
//! persisted, and a failed post-save activation restores the previous record
//! (mirroring the Slack setup rollback contract).

mod compensation;
mod service;
mod status;

pub use service::TelegramSetupService;
pub use status::{
    TELEGRAM_UPDATES_ROUTE_PATH, TelegramInstallationSetup, TelegramInstallationSetupStatus,
    TelegramInstallationSetupUpdate, TelegramSetupError,
};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
