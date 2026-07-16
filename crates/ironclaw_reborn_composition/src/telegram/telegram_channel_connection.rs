//! Per-user Telegram connection status: the seam the extension lifecycle
//! consults to decide whether in-chat activation must park on the pairing
//! gate. Filled by the telegram host mounts at serve time; an unfilled slot
//! means the deployment has not enabled the Telegram host at runtime.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use ironclaw_host_api::UserId;

use crate::telegram::telegram_pairing::{TelegramPairingError, TelegramPairingService};

/// Narrow pairedness probe (the lifecycle port must not hold the full pairing
/// service surface).
#[async_trait]
pub(crate) trait TelegramPairedStatusSource: Send + Sync + std::fmt::Debug {
    async fn telegram_paired(&self, user_id: &UserId) -> Result<bool, TelegramPairingError>;
}

#[async_trait]
impl TelegramPairedStatusSource for TelegramPairingService {
    async fn telegram_paired(&self, user_id: &UserId) -> Result<bool, TelegramPairingError> {
        Ok(self.status_for(user_id).await?.connected)
    }
}

/// Deferred pointer filled by [`crate::telegram`] host mounts once they are
/// built (mirrors the `SlackPersonalSetupServiceSlot` pattern).
#[derive(Clone, Default, Debug)]
pub(crate) struct TelegramPairedStatusSlot(Arc<OnceLock<Arc<dyn TelegramPairedStatusSource>>>);

impl TelegramPairedStatusSlot {
    pub(crate) fn fill(&self, source: Arc<dyn TelegramPairedStatusSource>) -> bool {
        self.0.set(source).is_ok()
    }

    pub(crate) fn get(&self) -> Option<Arc<dyn TelegramPairedStatusSource>> {
        self.0.get().map(Arc::clone)
    }
}
