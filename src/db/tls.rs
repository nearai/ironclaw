//! TLS connector factory for PostgreSQL connections.
//!
//! Builds a [`deadpool_postgres::Pool`] with the appropriate TLS connector
//! based on the configured [`SslMode`].  Uses `rustls` with system root
//! certificates, falling back to Mozilla's bundled roots via `webpki-roots`
//! when the system store is empty (common in minimal container images).

use deadpool_postgres::{Pool, Runtime};
use thiserror::Error;
use tokio_postgres::NoTls;
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::config::SslMode;

#[derive(Debug, Error)]
pub enum CreatePoolError {
    #[error("{0}")]
    Pool(#[from] deadpool_postgres::CreatePoolError),
    #[error("postgres TLS configuration failed: {0}")]
    TlsConfig(#[from] rustls::Error),
}

/// Build a rustls-based TLS connector.
///
/// Tries the platform's native certificate store first. If that yields zero
/// certificates (slim container images, missing ca-certificates package),
/// falls back to Mozilla's root certificates bundled via `webpki-roots`.
fn make_rustls_connector() -> Result<MakeRustlsConnect, rustls::Error> {
    let mut root_store = rustls::RootCertStore::empty();

    // Try native certs first.
    let native = rustls_native_certs::load_native_certs();
    for e in &native.errors {
        tracing::warn!("error loading system root certs: {e}");
    }
    for cert in native.certs {
        if let Err(e) = root_store.add(cert) {
            tracing::warn!("skipping invalid system root cert: {e}");
        }
    }

    // Fall back to bundled Mozilla roots when the system store is empty.
    if root_store.is_empty() {
        tracing::info!("no system root certificates found, using bundled Mozilla roots");
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }

    // Pick the ring crypto provider (same one reqwest uses).
    let config = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()?
    .with_root_certificates(root_store)
    .with_no_client_auth();
    Ok(MakeRustlsConnect::new(config))
}

/// Create a [`deadpool_postgres::Pool`] with the appropriate TLS connector.
///
/// - `Disable` → plain TCP (no TLS)
/// - `Prefer` / `Require` → rustls with system or bundled root certificates
///
/// **Note:** `Prefer` and `Require` currently behave identically — both
/// provide a TLS connector and will fail if the server rejects the TLS
/// handshake.  True `prefer` semantics (retry without TLS on failure)
/// would require reconnection logic that tokio-postgres does not provide
/// out of the box.  The three-variant enum is kept for forward-compatibility
/// and familiarity with libpq's `sslmode` parameter.
pub fn create_pool(
    config: &deadpool_postgres::Config,
    ssl_mode: SslMode,
) -> Result<Pool, CreatePoolError> {
    match ssl_mode {
        SslMode::Disable => config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(CreatePoolError::from),
        SslMode::Prefer | SslMode::Require => {
            let tls = make_rustls_connector()?;
            config
                .create_pool(Some(Runtime::Tokio1), tls)
                .map_err(CreatePoolError::from)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_pool_disable_mode() {
        let mut config = deadpool_postgres::Config::new();
        config.url = Some("postgres://localhost/test".to_string());
        let pool = create_pool(&config, SslMode::Disable);
        assert!(pool.is_ok());
    }

    #[test]
    fn create_pool_prefer_mode() {
        let mut config = deadpool_postgres::Config::new();
        config.url = Some("postgres://localhost/test".to_string());
        let pool = create_pool(&config, SslMode::Prefer);
        assert!(pool.is_ok());
    }

    #[test]
    fn create_pool_require_mode() {
        let mut config = deadpool_postgres::Config::new();
        config.url = Some("postgres://localhost/test".to_string());
        let pool = create_pool(&config, SslMode::Require);
        assert!(pool.is_ok());
    }
}
