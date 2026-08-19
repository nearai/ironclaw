//! Bounded custody for in-progress Telegram device logins.
//!
//! A login is bound to its MTProto connection, so the connection and mutable
//! protocol phase must survive between product requests. This module keeps
//! that parked state bounded, TTL-aware, and serializable per flow.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use grammers_client::client::{LoginToken, PasswordToken};
use grammers_tl_types as tl;
use ironclaw_extension_contracts::device_link::{
    DeviceLinkError, DeviceLinkErrorCode, DeviceLinkFlowId,
};
use tracing::debug;

use crate::linked::transport::{MtprotoConnection, VendorOpKind};
use crate::linked::{MAX_PENDING_LINKS, PENDING_LINK_TTL};

use super::{INITIAL_EXPORT_BACKOFF, MAX_INPUT_ATTEMPTS};

/// How long an abort waits for `auth.logOut` before giving up on it.
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(10);

/// The bounded, TTL'd registry of in-progress links.
///
/// A [`std::sync::Mutex`], so it cannot be held across an `await` even by
/// accident; every async abort path takes the `Arc` out first and works on it
/// outside the lock.
#[derive(Default)]
pub(super) struct PendingLinks {
    entries: Mutex<HashMap<DeviceLinkFlowId, Arc<PendingLink>>>,
}

impl PendingLinks {
    /// Whether one more link would fit. Replacing an existing flow always
    /// fits: it consumes no additional slot.
    pub(super) fn check_capacity(&self, flow_id: &DeviceLinkFlowId) -> Result<(), DeviceLinkError> {
        let entries = self.lock()?;
        if entries.len() >= MAX_PENDING_LINKS && !entries.contains_key(flow_id) {
            return Err(DeviceLinkError::Vendor {
                code: DeviceLinkErrorCode::RateLimited,
                restartable: true,
            });
        }
        Ok(())
    }

    pub(super) fn insert(
        &self,
        flow_id: DeviceLinkFlowId,
        link: Arc<PendingLink>,
    ) -> Result<(), DeviceLinkError> {
        self.check_capacity(&flow_id)?;
        self.lock()?.insert(flow_id, link);
        Ok(())
    }

    pub(super) fn get(&self, flow_id: &DeviceLinkFlowId) -> Option<Arc<PendingLink>> {
        self.entries.lock().ok()?.get(flow_id).map(Arc::clone)
    }

    pub(super) fn remove(&self, flow_id: &DeviceLinkFlowId) -> Option<Arc<PendingLink>> {
        self.entries.lock().ok()?.remove(flow_id)
    }

    pub(super) fn take_expired(&self) -> Vec<Arc<PendingLink>> {
        let Ok(mut entries) = self.entries.lock() else {
            return Vec::new();
        };
        let expired = entries
            .iter()
            .filter(|(_, link)| link.created_at.elapsed() >= PENDING_LINK_TTL)
            .map(|(flow_id, _)| flow_id.clone())
            .collect::<Vec<_>>();
        expired
            .into_iter()
            .filter_map(|flow_id| entries.remove(&flow_id))
            .collect()
    }

    pub(super) fn drain(&self) -> Vec<Arc<PendingLink>> {
        let Ok(mut entries) = self.entries.lock() else {
            return Vec::new();
        };
        entries.drain().map(|(_, link)| link).collect()
    }

    fn lock(
        &self,
    ) -> Result<
        std::sync::MutexGuard<'_, HashMap<DeviceLinkFlowId, Arc<PendingLink>>>,
        DeviceLinkError,
    > {
        self.entries.lock().map_err(|_| DeviceLinkError::Internal {
            reason: "the pending device-link registry lock was poisoned",
        })
    }
}

/// One parked login and the mutex that serializes every vendor call against it.
pub(super) struct PendingLink {
    pub(super) connection: MtprotoConnection,
    pub(super) state: tokio::sync::Mutex<PendingState>,
    created_at: Instant,
}

impl PendingLink {
    pub(super) fn new(connection: MtprotoConnection) -> Self {
        Self {
            connection,
            state: tokio::sync::Mutex::new(PendingState::default()),
            created_at: Instant::now(),
        }
    }
}

pub(super) struct PendingState {
    pub(super) phase: PendingPhase,
    /// Telegram has issued an authorization for this device.
    pub(super) accepted: bool,
    attempts: u8,
    pub(super) export_backoff: Duration,
    /// `serverNow - localNow`, in seconds. Token expiry is server time.
    pub(super) server_offset: i64,
}

impl Default for PendingState {
    fn default() -> Self {
        Self {
            phase: PendingPhase::AwaitingIdentifier,
            accepted: false,
            attempts: 0,
            export_backoff: INITIAL_EXPORT_BACKOFF,
            server_offset: 0,
        }
    }
}

impl PendingState {
    pub(super) fn charge_attempt(&mut self) -> Result<(), DeviceLinkError> {
        self.attempts = self.attempts.saturating_add(1);
        if self.attempts > MAX_INPUT_ATTEMPTS {
            return Err(DeviceLinkError::Vendor {
                code: DeviceLinkErrorCode::RateLimited,
                restartable: true,
            });
        }
        Ok(())
    }
}

pub(super) enum PendingPhase {
    AwaitingScan,
    AwaitingIdentifier,
    AwaitingCode {
        token: Box<LoginToken>,
    },
    AwaitingPassword {
        token: Box<PasswordToken>,
    },
    Completed {
        account_label: String,
        vendor_user_ref: String,
    },
    Failed,
}

/// End a parked link, logging out first whenever Telegram authorized it.
pub(super) async fn abandon(link: Arc<PendingLink>) {
    let needs_logout = {
        let state = link.state.lock().await;
        should_logout_on_abandon(&state)
    };
    if !needs_logout {
        return;
    }
    let call = link
        .connection
        .invoke(&tl::functions::auth::LogOut {}, VendorOpKind::Write);
    match tokio::time::timeout(LOGOUT_TIMEOUT, call).await {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => debug!("logging out an abandoned telegram device link failed"),
        Err(_) => debug!("logging out an abandoned telegram device link timed out"),
    }
}

pub(super) fn should_logout_on_abandon(state: &PendingState) -> bool {
    state.accepted
}
