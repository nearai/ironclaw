//! Telegram pairing: IronClaw-issued codes, deep-link consume, identity
//! binding, and blocked-run resume dispatch.
//!
//! Direction is web→telegram (WebGeneratedCode): IronClaw mints a short-lived
//! single-use code presented as `https://t.me/<bot>?start=<CODE>`; the webhook
//! consumes it (`/start <CODE>` or a bare live code) and binds the sending
//! Telegram account to the code's Reborn user. Codes expire; gates don't —
//! the parked `BlockedAuth` run is provider-keyed (`telegram`), so pairing
//! with the n-th rotated code still resumes it via the standard
//! auth-continuation fan-out.

mod code;
mod service;
mod status;

pub use service::TelegramPairingService;
pub use status::{
    PAIRING_CODE_ALPHABET, PAIRING_CODE_LEN, PAIRING_TTL_MINUTES, PairingCode, PairingCodeError,
    PairingConsumeOutcome, PairingIssue, RemovedTelegramBinding, TelegramBindingError,
    TelegramDmTarget, TelegramPairingError, TelegramPairingRecord, TelegramPairingStatus,
};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) use tests::pairing_test_support;
