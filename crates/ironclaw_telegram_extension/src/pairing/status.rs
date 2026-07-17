use chrono::{DateTime, Utc};
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_product_adapters::AdapterInstallationId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::setup::TelegramSetupError;

pub const PAIRING_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub const PAIRING_CODE_LEN: usize = 8;
pub const PAIRING_TTL_MINUTES: i64 = 15;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramPairingRecord {
    pub code: String,
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub installation_id: AdapterInstallationId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

impl TelegramPairingRecord {
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.consumed_at.is_none() && self.expires_at > now
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TelegramPairingError {
    #[error("telegram pairing store unavailable: {reason}")]
    StoreUnavailable { reason: String },
    #[error("telegram is not configured by an administrator yet")]
    NotConfigured,
    #[error("telegram setup unavailable: {reason}")]
    Setup { reason: String },
    #[error("pairing continuation dispatch failed: {reason}")]
    ContinuationDispatch { reason: String },
}

impl From<TelegramSetupError> for TelegramPairingError {
    fn from(error: TelegramSetupError) -> Self {
        TelegramPairingError::Setup {
            reason: error.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TelegramBindingError {
    #[error("telegram binding store unavailable: {reason}")]
    StoreUnavailable { reason: String },
    #[error("this telegram account is already paired to another user")]
    AlreadyBoundToOtherUser,
}

/// A binding removed by the concrete host state's user-scoped unbind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedTelegramBinding {
    pub provider_user_id: String,
    /// `None` only when the stored record was unreadable at removal time; the
    /// conditional pairing cleanup then fails safe (owner-changed no-op).
    pub epoch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramDmTarget {
    pub user_id: UserId,
    pub chat_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairingIssue {
    pub code: String,
    pub deep_link: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TelegramPairingStatus {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<PairingIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingConsumeOutcome {
    Paired { user_id: UserId },
    AlreadyPairedSameUser { user_id: UserId },
    AlreadyBoundToOtherUser,
    ExpiredOrUnknown,
}
