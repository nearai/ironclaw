//! Boot-time configuration for the standalone `ironclaw-reborn` binary.
//!
//! All inputs come from environment variables — no shared `Config` type with
//! the v1 agent. Operators run this binary against its own DB (or, in dev,
//! point it at the same one as v1).

use std::env;
use std::net::SocketAddr;

use secrecy::SecretString;

use crate::error::HostError;

/// Selects which storage backend the host wires up. Only one is active at a
/// time. `Postgres` takes precedence when both are configured.
#[derive(Debug, Clone)]
pub enum StorageBackend {
    #[cfg(feature = "libsql")]
    LibSql { path: String },
    #[cfg(feature = "postgres")]
    Postgres { url: String },
}

#[derive(Debug, Clone)]
pub struct HostConfig {
    /// Address to bind the axum webhook server on.
    pub listen_addr: SocketAddr,
    /// Storage backend wiring.
    pub storage: StorageBackend,
    /// Reborn installation id (one process = one install for the tracer).
    pub installation_id: String,
    /// Telegram bot token. Wrapped in `SecretString` so it zeroizes on drop
    /// and accidental `Debug` / `Display` prints reveal `[REDACTED]` rather
    /// than the literal token. The token still ends up cloned into
    /// `StaticCredentialResolver` (which holds a plain `String`) for the
    /// lifetime of the runner — fully eliminating that residual exposure
    /// requires re-reading through `EgressCredentialResolver` on each
    /// resolve, which zmanian's review on PR #3590 (item #3) flags as a
    /// major-tier follow-up before non-default-off rollout.
    pub telegram_bot_token: SecretString,
    /// Telegram webhook shared secret (sent in `X-Telegram-Bot-Api-Secret-Token`).
    pub telegram_webhook_secret: SecretString,
    /// Optional tenant id override (defaults to `tenant_default`).
    pub tenant_id: String,
    /// Optional agent id override (defaults to `agent_default`).
    pub agent_id: String,
}

impl HostConfig {
    pub fn from_env() -> Result<Self, HostError> {
        let listen_addr = env::var("IRONCLAW_REBORN_LISTEN_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8090".to_string())
            .parse()
            .map_err(|e| HostError::Config(format!("invalid IRONCLAW_REBORN_LISTEN_ADDR: {e}")))?;

        let storage = resolve_storage()?;

        let installation_id = env::var("REBORN_TELEGRAM_V2_INSTALLATION_ID")
            .unwrap_or_else(|_| "default".to_string());

        let telegram_bot_token = env::var("TELEGRAM_BOT_TOKEN")
            .map_err(|_| HostError::Config("TELEGRAM_BOT_TOKEN must be set".into()))?
            .into();
        let telegram_webhook_secret = env::var("TELEGRAM_WEBHOOK_SECRET")
            .map_err(|_| HostError::Config("TELEGRAM_WEBHOOK_SECRET must be set".into()))?
            .into();

        let tenant_id = env::var("REBORN_TENANT_ID").unwrap_or_else(|_| "tenant_default".into());
        let agent_id = env::var("REBORN_AGENT_ID").unwrap_or_else(|_| "agent_default".into());

        Ok(Self {
            listen_addr,
            storage,
            installation_id,
            telegram_bot_token,
            telegram_webhook_secret,
            tenant_id,
            agent_id,
        })
    }
}

/// Env-var name an operator can set to *explicitly* opt into ephemeral
/// in-memory storage. Required for tests and dev loops; absent in any
/// production deployment that should survive a restart. Renamed from the
/// previous silent `:memory:` fallback because that fallback made it
/// impossible to tell a misconfigured production process apart from an
/// intentional dev session (Henry's PR #3590 review item #3).
const ALLOW_EPHEMERAL_ENV: &str = "IRONCLAW_REBORN_ALLOW_EPHEMERAL";

#[allow(unreachable_code)]
fn resolve_storage() -> Result<StorageBackend, HostError> {
    #[cfg(feature = "postgres")]
    if let Ok(url) = env::var("DATABASE_URL") {
        return Ok(StorageBackend::Postgres { url });
    }
    #[cfg(feature = "libsql")]
    if let Ok(path) = env::var("LIBSQL_PATH") {
        return Ok(StorageBackend::LibSql { path });
    }
    // Fail-closed default. The Reborn host's entire purpose is durable
    // idempotency + binding storage; a silent `:memory:` fallback would
    // break the idempotency contract on every restart without anyone
    // noticing. Operators who want ephemeral storage on purpose (tests,
    // dev loops) opt in explicitly via `IRONCLAW_REBORN_ALLOW_EPHEMERAL=1`.
    #[cfg(feature = "libsql")]
    if env::var(ALLOW_EPHEMERAL_ENV).is_ok_and(|v| !v.is_empty() && v != "0") {
        tracing::warn!(
            "Reborn host: using ephemeral in-memory libSQL storage because \
             {ALLOW_EPHEMERAL_ENV} is set. Ledger and bindings will be lost \
             on restart; not safe for production."
        );
        return Ok(StorageBackend::LibSql {
            path: ":memory:".to_string(),
        });
    }
    Err(HostError::Config(format!(
        "no durable storage configured — set DATABASE_URL (postgres) or \
         LIBSQL_PATH (libsql). For tests/dev, set {ALLOW_EPHEMERAL_ENV}=1 to \
         opt into in-memory storage."
    )))
}
