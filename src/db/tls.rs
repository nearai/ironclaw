//! TLS connector factory for PostgreSQL connections.
//!
//! Builds a [`deadpool_postgres::Pool`] with the appropriate TLS connector
//! based on the configured [`SslMode`].  Uses `rustls` with system root
//! certificates — the same TLS stack that `reqwest` already uses for HTTP.

use deadpool_postgres::{Pool, Runtime};
use tokio_postgres::NoTls;
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::config::SslMode;

/// Build a rustls-based TLS connector using the platform's root certificate store.
fn make_rustls_connector() -> MakeRustlsConnect {
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
        tracing::error!("no system root certificates found -- TLS connections will fail");
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    MakeRustlsConnect::new(config)
}

/// Create a [`deadpool_postgres::Pool`] with the appropriate TLS connector.
///
/// - `Disable` → plain TCP (no TLS)
/// - `Prefer` / `Require` → rustls with system root certificates
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
) -> Result<Pool, deadpool_postgres::CreatePoolError> {
    match ssl_mode {
        SslMode::Disable => config.create_pool(Some(Runtime::Tokio1), NoTls),
        SslMode::Prefer | SslMode::Require => {
            let tls = make_rustls_connector();
            config.create_pool(Some(Runtime::Tokio1), tls)
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
        // Should succeed — pool is created lazily, no actual connection needed.
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
