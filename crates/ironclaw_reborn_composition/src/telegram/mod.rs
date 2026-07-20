//! Reborn Telegram channel host wiring (single `telegram` extension).
//!
//! The Telegram host domain lives in the `ironclaw_telegram_extension` crate;
//! this module keeps only the composition point that assembles the crate's
//! services from an already-built [`crate::RebornRuntime`] and hands the
//! route fragments to serve. Always compiled in; the Telegram host is enabled
//! at runtime via `[telegram].enabled` / `IRONCLAW_REBORN_TELEGRAM_ENABLED`.

pub(crate) mod telegram_host_beta;
