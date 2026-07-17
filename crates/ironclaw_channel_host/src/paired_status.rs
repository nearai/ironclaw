//! Per-user channel pairedness probe: the seam an extension lifecycle
//! consults to decide whether in-chat activation of a pairing-gated channel
//! extension must park on the pairing gate.
//!
//! The slot is filled by the owning channel host's mounts at serve time; an
//! unfilled slot means the deployment never enabled that channel host, so
//! activation fails closed instead of parking a run nothing can resume.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use ironclaw_host_api::UserId;
use thiserror::Error;

/// The pairedness read failed (backend outage, store error). The lifecycle
/// maps this to a temporary activation failure; it never parks a gate on it.
#[derive(Debug, Error)]
#[error("channel paired-status read failed: {reason}")]
pub struct ChannelPairedStatusError {
    pub reason: String,
}

impl ChannelPairedStatusError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Narrow pairedness probe (the lifecycle port must not hold the full pairing
/// service surface).
#[async_trait]
pub trait ChannelPairedStatusSource: Send + Sync + std::fmt::Debug {
    async fn paired(&self, user_id: &UserId) -> Result<bool, ChannelPairedStatusError>;
}

/// Deferred pointer filled by a channel host's mounts once they are built
/// (mirrors composition's `SlackPersonalSetupServiceSlot` pattern).
#[derive(Clone, Default, Debug)]
pub struct ChannelPairedStatusSlot(Arc<OnceLock<Arc<dyn ChannelPairedStatusSource>>>);

impl ChannelPairedStatusSlot {
    pub fn fill(&self, source: Arc<dyn ChannelPairedStatusSource>) -> bool {
        self.0.set(source).is_ok()
    }

    pub fn get(&self) -> Option<Arc<dyn ChannelPairedStatusSource>> {
        self.0.get().map(Arc::clone)
    }
}
