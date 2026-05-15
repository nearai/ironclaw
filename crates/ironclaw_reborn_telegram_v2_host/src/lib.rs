//! Reborn Telegram v2 webhook host.
//!
//! Today this layer terminates inbound at the durable ledger / binding write
//! and acks 200 to Telegram. The reply path is intentionally stubbed: there
//! is no Reborn agent loop in `src/` yet (PRs #3544 / #3550 / #3586 open).
//! The tracer's purpose is to lock down the inbound contract — webhook auth,
//! parse, idempotency, binding persistence, ledger settlement — so swapping
//! in the real loop once it ships is a one-line change in `boot.rs`.
//!
//! ## Wiring
//!
//! This crate is library-only. The `ironclaw-reborn` binary (in
//! `ironclaw_reborn_cli`) calls [`serve_from_env`] from its `run` subcommand
//! when [`telegram_v2_configured_in_env`] returns `true`. The v1 `ironclaw`
//! agent has zero awareness this crate exists.

#[cfg(not(any(feature = "libsql", feature = "postgres")))]
compile_error!(
    "ironclaw_reborn_telegram_v2_host requires at least one of the `libsql` or `postgres` features"
);

pub mod boot;
pub mod composition;
pub mod config;
pub mod error;
pub mod inbound_turn;
pub mod migrations;
pub mod router;

#[cfg(feature = "libsql")]
use std::sync::Arc;

use tracing_subscriber::EnvFilter;

use crate::composition::BackendHandles;
use crate::config::{HostConfig, StorageBackend};
use crate::error::HostError;

/// Returns `true` if Telegram v2 is configured in the environment.
///
/// Callers (the `ironclaw-reborn` CLI) use this to decide whether to enter
/// the long-lived webhook serve loop. Absence is silent — the CLI just falls
/// through to its existing behavior. Full validation (webhook secret,
/// durable storage, listen addr) happens inside [`HostConfig::from_env`]
/// and surfaces as a typed [`HostError`] when serve actually starts.
pub fn telegram_v2_configured_in_env() -> bool {
    std::env::var("TELEGRAM_BOT_TOKEN").is_ok()
}

/// Initialize a tracing subscriber for the long-lived serve loop.
///
/// Idempotent across this process: callers should invoke this exactly once
/// before [`serve`] / [`serve_from_env`]. Honors `RUST_LOG`; defaults to
/// `ironclaw_reborn=info,info` when unset.
pub fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("ironclaw_reborn=info,info")),
        )
        .init();
}

/// Read [`HostConfig`] from env and run the webhook serve loop.
pub async fn serve_from_env() -> Result<(), HostError> {
    let config = HostConfig::from_env()?;
    serve(config).await
}

/// Connect storage, run migrations, build the router, and serve axum on
/// `config.listen_addr` until the listener closes.
pub async fn serve(config: HostConfig) -> Result<(), HostError> {
    tracing::info!(
        listen_addr = %config.listen_addr,
        installation_id = %config.installation_id,
        "ironclaw-reborn starting"
    );

    let handles = connect_backend(&config.storage).await?;
    let artifacts = boot::boot(handles, &config).await?;

    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await
        .map_err(|e| HostError::Startup(format!("bind {}: {e}", config.listen_addr)))?;
    tracing::info!(addr = %config.listen_addr, "ironclaw-reborn listening");
    axum::serve(listener, artifacts.router)
        .await
        .map_err(|e| HostError::Startup(format!("axum serve: {e}")))?;

    Ok(())
}

async fn connect_backend(storage: &StorageBackend) -> Result<BackendHandles, HostError> {
    match storage {
        #[cfg(feature = "libsql")]
        StorageBackend::LibSql { path } => {
            let db = if path == ":memory:" {
                libsql::Builder::new_local(":memory:").build().await
            } else {
                libsql::Builder::new_local(path).build().await
            }
            .map_err(|e| HostError::Storage(format!("libsql open: {e}")))?;
            migrations::run_libsql_migrations(&db).await?;
            Ok(BackendHandles::LibSql(Arc::new(db)))
        }
        #[cfg(feature = "postgres")]
        StorageBackend::Postgres { url } => {
            use deadpool_postgres::{Config as PoolConfig, Runtime};
            let mut cfg = PoolConfig::new();
            cfg.url = Some(url.clone());
            let pool = cfg
                .create_pool(Some(Runtime::Tokio1), tokio_postgres::NoTls)
                .map_err(|e| HostError::Storage(format!("postgres pool build: {e}")))?;
            migrations::run_postgres_migrations(&pool).await?;
            Ok(BackendHandles::Postgres(pool))
        }
    }
}
