//! Reborn Telegram v2 webhook host.
//!
//! Today this layer terminates inbound at the durable ledger / binding write
//! and acks 200 to Telegram. The reply path is intentionally stubbed even
//! though the Reborn agent loop (PRs #3544 / #3550 / #3586) has now merged —
//! this PR is the inbound tracer, scoped to locking down the inbound
//! contract (webhook auth, parse, idempotency, binding persistence, ledger
//! settlement). Reply-path migration to `DefaultInboundTurnService` is a
//! follow-up so the inbound contract can soak in production before the
//! outbound path is wired up. The swap stays a one-line change in `boot.rs`.
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
pub mod host_egress;
pub mod inbound_turn;
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
    // Backend handles are opened here but their schema is owned by the
    // universal-FS dispatch fabric. `composition::build_*_layer` calls
    // `filesystem.run_migrations()` after wrapping the handle in the
    // matching `RootFilesystem` implementation — there is no separate
    // per-table SQL schema for product workflow persistence any more.
    match storage {
        #[cfg(feature = "libsql")]
        StorageBackend::LibSql { path } => {
            let db = if path == ":memory:" {
                libsql::Builder::new_local(":memory:").build().await
            } else {
                libsql::Builder::new_local(path).build().await
            }
            .map_err(|e| HostError::Storage(format!("libsql open: {e}")))?;
            Ok(BackendHandles::LibSql(Arc::new(db)))
        }
        #[cfg(feature = "postgres")]
        StorageBackend::Postgres { url } => {
            let pool = build_postgres_pool(url)?;
            Ok(BackendHandles::Postgres(pool))
        }
    }
}

#[cfg(feature = "postgres")]
fn build_postgres_pool(url: &str) -> Result<deadpool_postgres::Pool, HostError> {
    use deadpool_postgres::{Config as PoolConfig, Runtime};
    use tokio_postgres::config::SslMode;

    // Parse the URL once so we can branch on the requested `sslmode`. Managed
    // Postgres providers commonly require TLS via `sslmode=require`; passing
    // `NoTls` to those deployments fails the migration / pool connect even
    // though the rest of the crate is otherwise fully configured. Plain
    // `postgres://` URLs with no `sslmode` default to `Prefer`, which we
    // treat as TLS so the typical managed-Postgres URL works out of the box.
    let parsed: tokio_postgres::Config = url
        .parse()
        .map_err(|e| HostError::Storage(format!("postgres url parse: {e}")))?;
    let mut cfg = PoolConfig::new();
    cfg.url = Some(url.to_string());

    match parsed.get_ssl_mode() {
        SslMode::Disable => cfg
            .create_pool(Some(Runtime::Tokio1), tokio_postgres::NoTls)
            .map_err(|e| HostError::Storage(format!("postgres pool build: {e}"))),
        _ => {
            let tls = make_rustls_connector()?;
            cfg.create_pool(Some(Runtime::Tokio1), tls)
                .map_err(|e| HostError::Storage(format!("postgres pool build: {e}")))
        }
    }
}

/// Build a rustls-based TLS connector for Postgres.
///
/// Tries the system root store first (matching what `psql` /
/// `tokio-postgres` users normally expect). If empty — common in slim
/// container images — falls back to the Mozilla roots bundled via
/// `webpki-roots`. Mirrors the existing pattern in `src/db/tls.rs` and
/// `crates/ironclaw_reborn_event_store/src/lib.rs`.
#[cfg(feature = "postgres")]
fn make_rustls_connector() -> Result<tokio_postgres_rustls::MakeRustlsConnect, HostError> {
    let mut root_store = rustls::RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    for e in &native.errors {
        tracing::warn!("error loading system root certs: {e}");
    }
    for cert in native.certs {
        if let Err(e) = root_store.add(cert) {
            tracing::warn!("skipping invalid system root cert: {e}");
        }
    }
    if root_store.is_empty() {
        tracing::info!("no system root certs found, using bundled Mozilla roots");
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }
    let config = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .map_err(|e| HostError::Storage(format!("rustls protocol versions: {e}")))?
    .with_root_certificates(root_store)
    .with_no_client_auth();
    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}
