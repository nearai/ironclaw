//! Standalone Reborn Telegram v2 webhook host binary.
//!
//! Process model: one process per installation. The binary reads env vars,
//! connects to its own storage backend, runs the reborn-specific migrations,
//! builds the composition, and serves the webhook on a configurable port.
//!
//! The v1 `ironclaw` agent has zero awareness this binary exists. They share
//! no source files, no in-memory state, no Cargo dependencies (the v1 crate
//! depends on none of the optional Reborn product-layer crates).

#[cfg(feature = "libsql")]
use std::sync::Arc;

use ironclaw_reborn_telegram_v2_host::boot::boot;
use ironclaw_reborn_telegram_v2_host::composition::BackendHandles;
use ironclaw_reborn_telegram_v2_host::config::{HostConfig, StorageBackend};
use ironclaw_reborn_telegram_v2_host::error::HostError;
use ironclaw_reborn_telegram_v2_host::migrations;
use tracing_subscriber::EnvFilter;

#[cfg(not(any(feature = "libsql", feature = "postgres")))]
compile_error!("ironclaw-reborn requires at least one of the `libsql` or `postgres` features");

#[tokio::main]
async fn main() -> Result<(), HostError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("ironclaw_reborn=info,info")),
        )
        .init();

    let config = HostConfig::from_env()?;
    tracing::info!(
        listen_addr = %config.listen_addr,
        installation_id = %config.installation_id,
        "ironclaw-reborn starting"
    );

    let handles = connect_backend(&config.storage).await?;
    let artifacts = boot(handles, &config).await?;

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
