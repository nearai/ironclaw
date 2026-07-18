use chrono::{DateTime, Utc};
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_product_adapters::AdapterInstallationId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::setup::TelegramSetupError;

pub const PAIRING_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub const PAIRING_CODE_LEN: usize = 8;
pub const PAIRING_TTL_MINUTES: i64 = 15;

/// Canonical, validated pairing-code value used by persistence, minting, and
/// API projections. External text is normalized exactly once at ingress.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PairingCode(String);

impl PairingCode {
    pub fn parse(value: impl AsRef<str>) -> Result<Self, PairingCodeError> {
        let normalized = value.as_ref().trim().to_ascii_uppercase();
        if normalized.len() != PAIRING_CODE_LEN
            || !normalized
                .bytes()
                .all(|byte| PAIRING_CODE_ALPHABET.contains(&byte))
        {
            return Err(PairingCodeError);
        }
        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(super) fn generated(value: String) -> Self {
        debug_assert_eq!(value.len(), PAIRING_CODE_LEN);
        debug_assert!(
            value
                .bytes()
                .all(|byte| PAIRING_CODE_ALPHABET.contains(&byte))
        );
        Self(value)
    }
}

impl std::ops::Deref for PairingCode {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl std::fmt::Display for PairingCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for PairingCode {
    type Error = PairingCodeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<PairingCode> for String {
    fn from(value: PairingCode) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("invalid telegram pairing code")]
pub struct PairingCodeError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramPairingRecord {
    pub code: PairingCode,
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
    #[error("telegram pairing changed concurrently; retry")]
    ConcurrentUpdate,
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
    pub code: PairingCode,
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
