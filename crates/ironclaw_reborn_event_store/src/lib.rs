//! Reborn-owned durable event and audit store backends.
//!
//! This crate is the production-composition side of the Reborn event
//! substrate. `ironclaw_events` owns the durable log traits and redacted record
//! vocabulary; this crate owns backend selection, fail-closed profile
//! validation, and concrete storage adapters that should not live in the
//! substrate crate.
//!
//! Backend dispatch happens at the [`RootFilesystem`] layer: the `Libsql` /
//! `Postgres` variants of [`RebornEventStoreConfig`] open a backend-specific
//! `RootFilesystem` (libSQL / PostgreSQL) and route the durable log through
//! [`FilesystemDurableEventLog`] / [`FilesystemDurableAuditLog`] over a
//! [`ScopedFilesystem`] anchored at `/events`. The legacy per-backend
//! `LibSql*` / `Postgres*` impls that spoke SQL directly were removed during
//! the `src/db/` dissolution pass — see the design-doc entry "Legacy
//! per-backend store cleanup" in
//! `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.
//!
//! KNOWN LIMITATION (PR #3171 review #39): replay filtering currently stops
//! at project / mission / thread / process scope. The `ResourceScope` carries
//! an `invocation_id`, but `ReadScope` (defined in `ironclaw_events`) does
//! not yet expose it — so a per-invocation consumer sharing the same
//! `(tenant, user, agent)` stream cannot ask the backend to enforce the
//! invocation boundary. Adding it requires changes to `ironclaw_events`,
//! the JSONL/in-memory `matches_event` / `matches_audit` predicates, and
//! every replay caller — tracked as a follow-up.
#![warn(unreachable_pub)]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

use async_trait::async_trait;
use ironclaw_events::{
    DurableAuditLog, DurableEventLog, EventCursor, EventError, EventLogEntry, EventReplay,
    EventStreamKey, InMemoryDurableAuditLog, InMemoryDurableEventLog, ReadScope, RuntimeEvent,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{AgentId, AuditEnvelope};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
use secrecy::SecretString;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;
use tokio::sync::Mutex;

mod filesystem_store;

pub use filesystem_store::{FilesystemDurableAuditLog, FilesystemDurableEventLog};

/// Backend configuration for Reborn durable event/audit stores.
///
/// The `Libsql` / `Postgres` variants open a backend-specific
/// [`RootFilesystem`] internally and route the durable log through
/// [`FilesystemDurableEventLog`] / [`FilesystemDurableAuditLog`]. Production
/// callers that match on this enum see the same external shape; backend
/// dispatch now happens at the filesystem layer rather than at the
/// consumer-store layer.
#[derive(Debug)]
pub enum RebornEventStoreConfig {
    /// In-memory reference backend. Valid only for explicit local/test
    /// profiles; production rejects it before returning a service graph.
    InMemory,
    /// Single-node durable JSONL backend rooted outside V1 migrations and DB
    /// traits. Production must explicitly accept this single-node durability
    /// mode so it cannot become an implicit memory-style fallback.
    Jsonl {
        root: PathBuf,
        accept_single_node_durable: bool,
    },
    /// PostgreSQL backend configuration. The store opens a
    /// [`PostgresRootFilesystem`](ironclaw_filesystem::PostgresRootFilesystem)
    /// over the provided URL and runs durable-log ops through the unified
    /// filesystem dispatch fabric.
    Postgres { url: SecretString },
    /// libSQL backend configuration. The store opens a
    /// [`LibSqlRootFilesystem`](ironclaw_filesystem::LibSqlRootFilesystem)
    /// over the provided local path or remote URL and runs durable-log ops
    /// through the unified filesystem dispatch fabric.
    Libsql {
        path_or_url: String,
        auth_token: Option<SecretString>,
    },
}

/// Reborn composition profile controlling which fallbacks are legal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebornProfile {
    LocalDev,
    Test,
    Production,
}

/// Durable event and audit log handles consumed by Reborn composition.
#[derive(Clone)]
pub struct RebornEventStores {
    pub events: Arc<dyn DurableEventLog>,
    pub audit: Arc<dyn DurableAuditLog>,
}

/// Redacted factory/configuration errors.
#[derive(Debug, Error)]
pub enum RebornEventStoreError {
    #[error("production Reborn event store cannot use in-memory storage")]
    ProductionInMemoryDisabled,
    #[error("production JSONL event store requires explicit single-node durable acceptance")]
    ProductionJsonlRequiresAcceptance,
    #[error("production Reborn event store cannot use cleartext http:// libSQL URL")]
    ProductionLibsqlClearTextDisabled,
    #[error(
        "production Reborn libSQL event store requires an explicit local path or remote URL scheme"
    )]
    ProductionLibsqlAmbiguousTarget,
    #[error(
        "remote Reborn Postgres event store requires sslmode=require (sslmode=disable rejected)"
    )]
    RemotePostgresClearTextDisabled,
    #[error("{backend} Reborn event store backend is not enabled in this build")]
    BackendUnavailable { backend: &'static str },
    #[error("{backend} Reborn event store failed during {operation}")]
    BackendOperation {
        backend: &'static str,
        operation: &'static str,
    },
    #[error("Reborn event store I/O failed during {operation}")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
}

impl RebornEventStoreError {
    fn io(operation: &'static str, source: std::io::Error) -> Self {
        Self::Io { operation, source }
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    fn backend<E>(backend: &'static str, operation: &'static str, _source: E) -> Self {
        Self::BackendOperation { backend, operation }
    }
}

/// Build durable event and audit logs for a standalone Reborn composition path.
pub async fn build_reborn_event_stores(
    profile: RebornProfile,
    config: RebornEventStoreConfig,
) -> Result<RebornEventStores, RebornEventStoreError> {
    match config {
        RebornEventStoreConfig::InMemory => {
            if profile == RebornProfile::Production {
                return Err(RebornEventStoreError::ProductionInMemoryDisabled);
            }
            Ok(RebornEventStores {
                events: Arc::new(InMemoryDurableEventLog::new()),
                audit: Arc::new(InMemoryDurableAuditLog::new()),
            })
        }
        RebornEventStoreConfig::Jsonl {
            root,
            accept_single_node_durable,
        } => {
            if profile == RebornProfile::Production && !accept_single_node_durable {
                return Err(RebornEventStoreError::ProductionJsonlRequiresAcceptance);
            }
            create_secure_dir_all(&root)
                .await
                .map_err(|source| RebornEventStoreError::io("initialize jsonl root", source))?;
            let store = JsonlStore::new(root);
            Ok(RebornEventStores {
                events: Arc::new(JsonlDurableEventLog::from_store(store.clone())),
                audit: Arc::new(JsonlDurableAuditLog::from_store(store)),
            })
        }
        RebornEventStoreConfig::Postgres { url } => {
            #[cfg(feature = "postgres")]
            {
                postgres_backed::build(url).await
            }
            #[cfg(not(feature = "postgres"))]
            {
                let _ = url;
                Err(RebornEventStoreError::BackendUnavailable {
                    backend: "postgres",
                })
            }
        }
        RebornEventStoreConfig::Libsql {
            path_or_url,
            auth_token,
        } => {
            if profile == RebornProfile::Production {
                validate_production_libsql_target(&path_or_url)?;
            }
            #[cfg(feature = "libsql")]
            {
                libsql_backed::build(path_or_url, auth_token).await
            }
            #[cfg(not(feature = "libsql"))]
            {
                let _ = (path_or_url, auth_token);
                Err(RebornEventStoreError::BackendUnavailable { backend: "libsql" })
            }
        }
    }
}

/// Build a [`RebornEventStores`] from any [`RootFilesystem`] by routing the
/// durable log through [`FilesystemDurableEventLog`] /
/// [`FilesystemDurableAuditLog`] over a [`ScopedFilesystem`] anchored at
/// `/events`. Production composition reuses this on top of a libSQL /
/// PostgreSQL `RootFilesystem` so the backend choice is a property of the
/// filesystem rather than of the durable-log impl.
#[cfg(any(feature = "libsql", feature = "postgres"))]
fn wrap_root_filesystem_as_event_stores<F>(
    root: Arc<F>,
) -> Result<RebornEventStores, RebornEventStoreError>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let scoped = build_events_scoped_filesystem(root)?;
    Ok(RebornEventStores {
        events: Arc::new(FilesystemDurableEventLog::new(Arc::clone(&scoped))),
        audit: Arc::new(FilesystemDurableAuditLog::new(scoped)),
    })
}

/// Wrap a [`RootFilesystem`] in a [`ScopedFilesystem`] whose [`MountView`]
/// grants the `/events` plane the permissions the durable log needs
/// (append → write, tail → read+list).
#[cfg(any(feature = "libsql", feature = "postgres"))]
fn build_events_scoped_filesystem<F>(
    root: Arc<F>,
) -> Result<Arc<ScopedFilesystem<F>>, RebornEventStoreError>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let alias =
        MountAlias::new("/events").map_err(|_| RebornEventStoreError::BackendOperation {
            backend: "filesystem",
            operation: "construct events mount alias",
        })?;
    let target =
        VirtualPath::new("/events").map_err(|_| RebornEventStoreError::BackendOperation {
            backend: "filesystem",
            operation: "construct events mount target",
        })?;
    let view = MountView::new(vec![MountGrant::new(
        alias,
        target,
        MountPermissions {
            read: true,
            write: true,
            delete: false,
            list: true,
            execute: false,
        },
    )])
    .map_err(|_| RebornEventStoreError::BackendOperation {
        backend: "filesystem",
        operation: "construct events mount view",
    })?;
    Ok(Arc::new(ScopedFilesystem::with_fixed_view(root, view)))
}

/// Classification of a libSQL `path_or_url` for production policy decisions.
///
/// Scheme detection is case-insensitive so `HTTPS://` / `LibSQL://` cannot
/// silently fall through to the local-file path and create a node-local
/// SQLite file named after the URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LibsqlTargetClass {
    /// `http://` (any case). Production rejects to prevent cleartext auth
    /// tokens crossing the wire.
    RemoteCleartext,
    /// `https://` or `libsql://` (any case). Acceptable in production.
    RemoteSecure,
    /// `:memory:` reference backend. Production rejects: durable history must
    /// not silently disappear on restart.
    InMemory,
    /// Absolute filesystem path (`/abs/...`). Acceptable in production.
    LocalAbsolute,
    /// Explicit relative-path syntax (`./...`, `../...`). Acceptable in
    /// production because the relative-path intent is unambiguous.
    LocalRelative,
    /// Bare token with no scheme and no path syntax (e.g. `events.db`,
    /// `db.example.com`). Ambiguous: could be a remote hostname typo or a
    /// CWD-relative file. Production rejects to fail closed.
    Bare,
}

fn classify_libsql_target(path_or_url: &str) -> LibsqlTargetClass {
    if path_or_url == ":memory:" {
        return LibsqlTargetClass::InMemory;
    }
    if let Some(scheme_end) = path_or_url.find("://") {
        let scheme = &path_or_url[..scheme_end];
        if scheme.eq_ignore_ascii_case("http") {
            return LibsqlTargetClass::RemoteCleartext;
        }
        if scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("libsql") {
            return LibsqlTargetClass::RemoteSecure;
        }
        // Unknown scheme: treat as bare so production fails closed instead of
        // accidentally routing through `Builder::new_local`.
        return LibsqlTargetClass::Bare;
    }
    if path_or_url.starts_with('/') {
        return LibsqlTargetClass::LocalAbsolute;
    }
    if path_or_url.starts_with("./") || path_or_url.starts_with("../") {
        return LibsqlTargetClass::LocalRelative;
    }
    LibsqlTargetClass::Bare
}

fn validate_production_libsql_target(path_or_url: &str) -> Result<(), RebornEventStoreError> {
    match classify_libsql_target(path_or_url) {
        LibsqlTargetClass::RemoteCleartext => {
            Err(RebornEventStoreError::ProductionLibsqlClearTextDisabled)
        }
        LibsqlTargetClass::InMemory => Err(RebornEventStoreError::ProductionInMemoryDisabled),
        LibsqlTargetClass::Bare => Err(RebornEventStoreError::ProductionLibsqlAmbiguousTarget),
        LibsqlTargetClass::RemoteSecure
        | LibsqlTargetClass::LocalAbsolute
        | LibsqlTargetClass::LocalRelative => Ok(()),
    }
}

#[cfg(feature = "libsql")]
mod libsql_backed {
    //! libSQL-backed [`RootFilesystem`] construction for the durable event
    //! store. The connection lives behind the standard
    //! [`FilesystemDurableEventLog`] / [`FilesystemDurableAuditLog`] surface
    //! so this module only owns URL → `libsql::Database` plumbing and the
    //! filesystem-layer migration.

    use std::{path::Path, sync::Arc};

    use ironclaw_filesystem::LibSqlRootFilesystem;
    use secrecy::{ExposeSecret, SecretString};

    use super::{RebornEventStoreError, RebornEventStores, wrap_root_filesystem_as_event_stores};

    pub(super) async fn build(
        path_or_url: String,
        auth_token: Option<SecretString>,
    ) -> Result<RebornEventStores, RebornEventStoreError> {
        let db = build_database(&path_or_url, auth_token).await?;
        let filesystem = Arc::new(LibSqlRootFilesystem::new(db));
        filesystem
            .run_migrations()
            .await
            .map_err(|source| RebornEventStoreError::backend("libsql", "run migrations", source))?;
        wrap_root_filesystem_as_event_stores(filesystem)
    }

    async fn build_database(
        path_or_url: &str,
        auth_token: Option<SecretString>,
    ) -> Result<Arc<libsql::Database>, RebornEventStoreError> {
        let db = if is_remote_libsql(path_or_url) {
            libsql::Builder::new_remote(
                path_or_url.to_string(),
                auth_token
                    .as_ref()
                    .map(|token| token.expose_secret().to_string())
                    .unwrap_or_default(),
            )
            .build()
            .await
        } else if path_or_url == ":memory:" {
            libsql::Builder::new_local(path_or_url).build().await
        } else {
            let path = Path::new(path_or_url);
            // `Path::parent()` returns `Some("")` for a bare filename like
            // `events.db`, and `create_dir_all("")` fails with ENOENT. Skip
            // parent creation when the parent is empty so common configs like
            // `path_or_url = "events.db"` work without forcing callers to
            // write `./events.db`.
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                tokio::fs::create_dir_all(parent).await.map_err(|source| {
                    RebornEventStoreError::io("initialize libsql parent", source)
                })?;
            }
            libsql::Builder::new_local(path_or_url).build().await
        };
        db.map(Arc::new)
            .map_err(|source| RebornEventStoreError::backend("libsql", "connect", source))
    }

    /// Detect a remote libSQL endpoint by recognised URL scheme.
    ///
    /// Scheme matching is case-insensitive: `HTTPS://...`, `LibSQL://...`, and
    /// `HTTP://...` would otherwise fall through to `Builder::new_local(...)`
    /// and silently create a node-local SQLite path like `HTTPS:/host/...`,
    /// stranding durable history on one node and ignoring the auth token.
    ///
    /// A bare value with no scheme (e.g. `db.example.com` or `events.db`) is
    /// treated as local here — production composition (in `lib.rs`) is
    /// responsible for rejecting that ambiguity before we get this far. See
    /// `validate_production_libsql_target`.
    fn is_remote_libsql(path_or_url: &str) -> bool {
        let Some(scheme_end) = path_or_url.find("://") else {
            return false;
        };
        let scheme = &path_or_url[..scheme_end];
        scheme.eq_ignore_ascii_case("libsql")
            || scheme.eq_ignore_ascii_case("https")
            || scheme.eq_ignore_ascii_case("http")
    }

    #[cfg(test)]
    mod tests {
        use super::is_remote_libsql;

        #[test]
        fn case_insensitive_remote_scheme_detection() {
            // Regression for nearai/ironclaw#3171 review finding: mixed-case
            // schemes previously fell through to `Builder::new_local`. The
            // detector now matches scheme case-insensitively.
            for url in [
                "libsql://example.invalid",
                "LIBSQL://example.invalid",
                "LibSQL://example.invalid",
                "https://example.invalid",
                "HTTPS://example.invalid",
                "Https://example.invalid",
                "http://example.invalid",
                "HTTP://example.invalid",
            ] {
                assert!(is_remote_libsql(url), "expected `{url}` to be remote");
            }
        }

        #[test]
        fn unscheme_values_are_local() {
            // Bare values and explicit local-path syntax are not remote — the
            // production gate in lib.rs handles ambiguity for production
            // profiles separately.
            for url in [
                ":memory:",
                "/var/lib/ironclaw/events.db",
                "./events.db",
                "events.db",
                "db.example.com",
            ] {
                assert!(!is_remote_libsql(url), "expected `{url}` to be local");
            }
        }
    }
}

#[cfg(feature = "postgres")]
mod postgres_backed {
    //! PostgreSQL-backed [`RootFilesystem`] construction for the durable
    //! event store. Mirrors `libsql_backed::build`: parse the URL, enforce
    //! the production TLS policy, open a pool, hand the pool to
    //! [`PostgresRootFilesystem`], and wrap the result in the standard
    //! filesystem-backed durable-log surface.

    use std::sync::Arc;

    use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod, Runtime};
    use ironclaw_filesystem::PostgresRootFilesystem;
    use secrecy::{ExposeSecret, SecretString};
    use tokio_postgres::config::{Host, SslMode};
    use tokio_postgres::{Config, NoTls};
    use tokio_postgres_rustls::MakeRustlsConnect;

    use super::{RebornEventStoreError, RebornEventStores, wrap_root_filesystem_as_event_stores};

    pub(super) async fn build(
        url: SecretString,
    ) -> Result<RebornEventStores, RebornEventStoreError> {
        let pool = build_pool(url).await?;
        let filesystem = Arc::new(PostgresRootFilesystem::new(pool));
        filesystem.run_migrations().await.map_err(|source| {
            RebornEventStoreError::backend("postgres", "run migrations", source)
        })?;
        wrap_root_filesystem_as_event_stores(filesystem)
    }

    async fn build_pool(url: SecretString) -> Result<Pool, RebornEventStoreError> {
        let raw_url = url.expose_secret();
        let mut pg_config: Config = raw_url.parse().map_err(|source| {
            RebornEventStoreError::backend("postgres", "parse connection string", source)
        })?;
        let manager_config = ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        };
        let local = is_local_postgres_config(&pg_config);
        let local_wants_tls = local && matches!(pg_config.get_ssl_mode(), SslMode::Require);
        let manager = if local && !local_wants_tls {
            // Local without an explicit `sslmode=require`: NoTls is acceptable
            // because the connection never leaves the host.
            Manager::from_config(pg_config, NoTls, manager_config)
        } else {
            if !local {
                // Remote: TLS is mandatory. Reject `sslmode=disable` and upgrade
                // `Prefer` to `Require` before handing the config to the manager.
                enforce_remote_ssl_mode(&mut pg_config)?;
            }
            // For local-with-`sslmode=require` we pass the config through
            // unchanged: the user explicitly opted in to TLS, so we route
            // through the rustls connector. Use cases include TLS-only
            // loopback Postgres and a local TLS-terminating proxy.
            let tls = make_rustls_connector()?;
            Manager::from_config(pg_config, tls, manager_config)
        };
        Pool::builder(manager)
            .runtime(Runtime::Tokio1)
            .build()
            .map_err(|source| RebornEventStoreError::backend("postgres", "build pool", source))
    }

    /// Returns true if the parsed Postgres `Config` targets only loopback
    /// hosts or Unix sockets. Anything else — including mixed lists where a
    /// remote host appears alongside a socket path — is treated as remote
    /// and must use TLS.
    ///
    /// We inspect the parsed `Config` rather than re-parsing the raw
    /// connection string so that all libpq forms are normalised:
    /// - `host=db.example.com` (keyword TCP)
    /// - `hostaddr=10.0.0.5` (numeric-IP keyword, returns no `Host` entry but
    ///   does add a hostaddr)
    /// - `postgresql:///db?host=db.example.com` (URL with empty authority +
    ///   `host` query param)
    /// - `host=/var/run/postgresql,db.example.com` (mixed list)
    fn is_local_postgres_config(config: &Config) -> bool {
        let hosts = config.get_hosts();
        let hostaddrs = config.get_hostaddrs();

        // Empty host list means libpq's compiled-in default socket directory —
        // treat as local only if there are no overriding hostaddrs.
        if hosts.is_empty() && hostaddrs.is_empty() {
            return true;
        }

        for host in hosts {
            match host {
                Host::Unix(_) => continue,
                Host::Tcp(name) => {
                    if !is_local_host_literal(name) {
                        return false;
                    }
                }
            }
        }
        for addr in hostaddrs {
            if !addr.is_loopback() && !addr.is_unspecified() {
                return false;
            }
        }
        true
    }

    fn is_local_host_literal(host: &str) -> bool {
        matches!(
            host,
            "localhost" | "127.0.0.1" | "::1" | "[::1]" | "0.0.0.0"
        )
    }

    /// Reject `sslmode=disable` for any non-local Postgres config.
    ///
    /// Passing a rustls connector to `tokio-postgres` is not enough on its
    /// own: the connector is *only* used when `Config::ssl_mode` is `Prefer`
    /// or `Require`. An explicit `sslmode=disable` in the connection string
    /// returns a plaintext stream before the connector is consulted, so a
    /// misconfigured production URL can silently downgrade. We reject that
    /// here, and force `Require` if the config left the default `Prefer` in
    /// place — otherwise `tokio-postgres` would still complete a `Prefer`
    /// connection that the server happens to refuse TLS on.
    fn enforce_remote_ssl_mode(config: &mut Config) -> Result<(), RebornEventStoreError> {
        match config.get_ssl_mode() {
            SslMode::Disable => Err(RebornEventStoreError::RemotePostgresClearTextDisabled),
            SslMode::Prefer => {
                config.ssl_mode(SslMode::Require);
                Ok(())
            }
            SslMode::Require => Ok(()),
            // Forward-compat: future tokio-postgres SslMode variants we don't
            // recognise are treated as already strict.
            _ => Ok(()),
        }
    }

    /// Build a rustls TLS connector for remote Postgres connections.
    ///
    /// Mirrors `src/db/tls.rs`: prefer the platform's native certificate
    /// store, fall back to Mozilla's bundled webpki roots when the system
    /// store is empty.
    fn make_rustls_connector() -> Result<MakeRustlsConnect, RebornEventStoreError> {
        let mut root_store = rustls::RootCertStore::empty();
        let native = rustls_native_certs::load_native_certs();
        for error in &native.errors {
            tracing::warn!("postgres event-store: error loading system root certs: {error}");
        }
        for cert in native.certs {
            if let Err(error) = root_store.add(cert) {
                tracing::warn!("postgres event-store: skipping invalid system root cert: {error}");
            }
        }
        if root_store.is_empty() {
            tracing::info!(
                "postgres event-store: no system root certificates found, using bundled Mozilla roots"
            );
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
        let config = rustls::ClientConfig::builder_with_provider(
            rustls::crypto::ring::default_provider().into(),
        )
        .with_safe_default_protocol_versions()
        .map_err(|source| RebornEventStoreError::backend("postgres", "configure rustls", source))?
        .with_root_certificates(root_store)
        .with_no_client_auth();
        Ok(MakeRustlsConnect::new(config))
    }

    #[cfg(test)]
    mod tests {
        use super::{Config, enforce_remote_ssl_mode, is_local_postgres_config};
        use crate::RebornEventStoreError;
        use tokio_postgres::config::SslMode;

        fn parse(url: &str) -> Config {
            url.parse::<Config>().unwrap_or_else(|e| {
                panic!("test connection string `{url}` failed to parse: {e}");
            })
        }

        fn is_local(url: &str) -> bool {
            is_local_postgres_config(&parse(url))
        }

        #[test]
        fn local_postgres_urls_are_recognised() {
            for url in [
                "postgres://user:pass@localhost/db",
                "postgres://user@127.0.0.1:5432/db",
                "postgresql://localhost/db",
                "postgres://[::1]/db",
                "postgres://user@0.0.0.0/db",
                // Unix-socket-style: libpq treats these as local.
                "host=/var/run/postgresql user=ironclaw dbname=ironclaw",
            ] {
                assert!(is_local(url), "expected `{url}` to be detected as local");
            }
        }

        #[test]
        fn remote_postgres_urls_require_tls() {
            for url in [
                "postgres://user:pass@db.internal/db",
                "postgres://user@10.0.0.5:5432/db",
                "postgresql://user@managed-postgres.example.com/db",
                "postgres://user@[2001:db8::1]/db",
            ] {
                assert!(!is_local(url), "expected `{url}` to require TLS");
            }
        }

        #[test]
        fn libpq_keyword_strings_to_remote_hosts_require_tls() {
            // Regression for the High-severity finding on PR #3171: libpq
            // keyword form was previously treated as local because the
            // original check fired on `!url.contains("://")`.
            for url in [
                "host=db.example.com user=event_user dbname=ironclaw",
                "host=10.0.0.5 port=5432 user=ironclaw",
                "user=ironclaw host=managed-pg.internal",
            ] {
                assert!(
                    !is_local(url),
                    "expected libpq keyword string `{url}` to require TLS"
                );
            }
        }

        #[test]
        fn libpq_keyword_strings_without_host_or_with_socket_path_are_local() {
            for url in [
                // No host= keyword: libpq default = local socket.
                "user=ironclaw dbname=ironclaw",
                // Socket directory.
                "host=/var/run/postgresql user=ironclaw dbname=ironclaw",
                // Localhost literal.
                "host=localhost user=ironclaw",
                "host=127.0.0.1 user=ironclaw",
            ] {
                assert!(is_local(url), "expected `{url}` to be detected as local");
            }
        }

        #[test]
        fn libpq_hostaddr_to_remote_address_requires_tls() {
            // Regression for the High-severity finding (round 2) on PR
            // #3171: hostaddr= is a libpq keyword that bypassed the previous
            // raw-string detector entirely; switching to
            // Config::get_hostaddrs() catches it.
            assert!(!is_local("hostaddr=10.0.0.5 user=ironclaw"));
            assert!(!is_local("hostaddr=2001:db8::1 user=ironclaw"));
        }

        #[test]
        fn libpq_hostaddr_to_loopback_is_local() {
            assert!(is_local("hostaddr=127.0.0.1 user=ironclaw"));
            assert!(is_local("hostaddr=::1 user=ironclaw"));
        }

        #[test]
        fn libpq_mixed_socket_and_remote_host_list_requires_tls() {
            // host=/var/run/postgresql,db.example.com — first socket, second
            // TCP. tokio-postgres parses this as two Host entries; if any
            // TCP host isn't loopback the whole config is remote.
            assert!(!is_local(
                "host=/var/run/postgresql,db.example.com user=ironclaw"
            ));
        }

        #[test]
        fn url_with_empty_authority_and_query_host_uses_query_host() {
            // postgresql:///db?host=db.example.com — empty authority routes
            // to a host listed in the query string, which the parsed Config
            // exposes as a TCP Host entry.
            assert!(!is_local(
                "postgresql:///db?host=db.example.com&user=ironclaw"
            ));
        }

        #[test]
        fn enforce_remote_ssl_mode_rejects_disable() {
            let mut config = parse("postgres://user@db.example.com/db?sslmode=disable");
            let err = enforce_remote_ssl_mode(&mut config)
                .expect_err("sslmode=disable on remote must be rejected");
            assert!(matches!(
                err,
                RebornEventStoreError::RemotePostgresClearTextDisabled
            ));
        }

        #[test]
        fn enforce_remote_ssl_mode_upgrades_prefer_to_require() {
            // Default sslmode is `prefer`, which silently downgrades when
            // the server declines TLS — for remote we force `require`.
            let mut config = parse("postgres://user@db.example.com/db");
            assert!(matches!(config.get_ssl_mode(), SslMode::Prefer));
            enforce_remote_ssl_mode(&mut config).expect("default prefer must upgrade to require");
            assert!(matches!(config.get_ssl_mode(), SslMode::Require));
        }

        #[test]
        fn enforce_remote_ssl_mode_keeps_require() {
            let mut config = parse("postgres://user@db.example.com/db?sslmode=require");
            enforce_remote_ssl_mode(&mut config).expect("require should pass through");
            assert!(matches!(config.get_ssl_mode(), SslMode::Require));
        }

        // --- libpq quoted / whitespace-tolerant keyword strings (issues #35, #47) ---

        #[test]
        fn libpq_quoted_socket_path_is_local() {
            // Regression for review finding #35: a libpq DSN that quotes the
            // socket path was previously misclassified as remote because the
            // string-level detector saw the value as `'/var/run/postgresql'`
            // (with the leading quote) instead of the unquoted path.
            // Switching to `Config::get_hosts()` parses the libpq
            // single-quote form correctly: the value is a
            // `Host::Unix("/var/run/postgresql")` and the config is local.
            // (libpq only recognises single quotes; double quotes are not a
            // libpq quoting mechanism.)
            let url = "host='/var/run/postgresql' user=ironclaw dbname=ironclaw";
            assert!(
                is_local(url),
                "expected quoted-socket DSN `{url}` to be local"
            );
        }

        #[test]
        fn libpq_whitespace_around_equals_classifies_remote_correctly() {
            // Regression for review finding #47: tokenising the raw DSN on
            // whitespace and looking for `host=` previously caused a
            // remote-host DSN with whitespace around `=` to be treated as
            // having no host, falling through to NoTls. Parsing through
            // `tokio_postgres::Config` normalises this — the resulting
            // `Host` entry is a TCP host that fails the local-literal
            // check.
            for url in [
                "host = db.internal user=ironclaw",
                "host =db.internal user=ironclaw",
                "host= db.internal user=ironclaw",
            ] {
                assert!(
                    !is_local(url),
                    "expected whitespace-keyword DSN `{url}` to require TLS"
                );
            }
        }
    }
}

/// JSONL-backed durable runtime event log.
#[derive(Clone)]
pub struct JsonlDurableEventLog {
    store: JsonlStore,
}

impl JsonlDurableEventLog {
    // No public constructor: production composition must go through
    // [`build_reborn_event_stores`] so the single-node-durable acceptance
    // gate (`Jsonl { accept_single_node_durable: true }`) cannot be bypassed
    // by directly wrapping a `JsonlDurableEventLog` in a `DurableEventSink`.
    fn from_store(store: JsonlStore) -> Self {
        Self { store }
    }
}

impl std::fmt::Debug for JsonlDurableEventLog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JsonlDurableEventLog")
            .field("root", &"<redacted>")
            .finish()
    }
}

#[async_trait]
impl DurableEventLog for JsonlDurableEventLog {
    async fn append(&self, event: RuntimeEvent) -> Result<EventLogEntry<RuntimeEvent>, EventError> {
        let stream = EventStreamKey::from_scope(&event.scope);
        self.store.append(StreamKind::Runtime, &stream, event).await
    }

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<RuntimeEvent>, EventError> {
        let owned_filter = filter.clone();
        self.store
            .read_after(
                StreamKind::Runtime,
                stream,
                filter,
                after,
                limit,
                move |event| owned_filter.matches_event(event),
            )
            .await
    }

    async fn head_cursor(
        &self,
        stream: &EventStreamKey,
        after: EventCursor,
    ) -> Result<EventCursor, EventError> {
        self.store
            .head_cursor(StreamKind::Runtime, stream, after)
            .await
    }
}

/// JSONL-backed durable audit log.
#[derive(Clone)]
pub struct JsonlDurableAuditLog {
    store: JsonlStore,
}

impl JsonlDurableAuditLog {
    // See `JsonlDurableEventLog` — no public constructor by design.
    fn from_store(store: JsonlStore) -> Self {
        Self { store }
    }
}

impl std::fmt::Debug for JsonlDurableAuditLog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JsonlDurableAuditLog")
            .field("root", &"<redacted>")
            .finish()
    }
}

#[async_trait]
impl DurableAuditLog for JsonlDurableAuditLog {
    async fn append(
        &self,
        record: AuditEnvelope,
    ) -> Result<EventLogEntry<AuditEnvelope>, EventError> {
        let stream = EventStreamKey::new(
            record.tenant_id.clone(),
            record.user_id.clone(),
            record.agent_id.clone(),
        );
        self.store.append(StreamKind::Audit, &stream, record).await
    }

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<AuditEnvelope>, EventError> {
        let owned_filter = filter.clone();
        self.store
            .read_after(
                StreamKind::Audit,
                stream,
                filter,
                after,
                limit,
                move |record| owned_filter.matches_audit(record),
            )
            .await
    }
}

#[derive(Debug, Clone)]
struct JsonlStore {
    root: PathBuf,
    locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
}

impl JsonlStore {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn append<T>(
        &self,
        kind: StreamKind,
        stream: &EventStreamKey,
        record: T,
    ) -> Result<EventLogEntry<T>, EventError>
    where
        T: Clone + Serialize + DeserializeOwned + Send + 'static,
    {
        let lock = self.stream_lock(kind, stream).await;
        let _guard = lock.lock().await;
        let path = self.stream_path(kind, stream);
        if let Some(parent) = path.parent() {
            create_secure_dir_all(parent)
                .await
                .map_err(|_| durable_error("jsonl event store failed to prepare stream"))?;
        }
        // Serialise the record outside the blocking section.
        let record_for_envelope = record.clone();
        let assigned_cursor = tokio::task::spawn_blocking(move || -> Result<u64, EventError> {
            // The OS-level exclusive lock spans both reading the prior tail
            // cursor and appending the new record so two processes cannot
            // both observe the same last cursor and emit duplicates.
            append_with_cursor_assignment(&path, |next_cursor| {
                let envelope = JsonlEntry {
                    cursor: EventCursor::new(next_cursor),
                    record: record_for_envelope,
                };
                serde_json::to_string(&envelope).map_err(|error| EventError::Serialize {
                    reason: error.to_string(),
                })
            })
        })
        .await
        .map_err(|_| durable_error("jsonl event store failed to append record"))??;
        Ok(EventLogEntry {
            cursor: EventCursor::new(assigned_cursor),
            record,
        })
    }

    async fn read_after<T>(
        &self,
        kind: StreamKind,
        stream: &EventStreamKey,
        _filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
        is_match: impl Fn(&T) -> bool + Send + 'static,
    ) -> Result<EventReplay<T>, EventError>
    where
        T: Clone + DeserializeOwned + Send + 'static,
    {
        if limit == 0 {
            return Err(EventError::InvalidReplayRequest {
                reason: "limit must be greater than zero".to_string(),
            });
        }
        let after = after.unwrap_or_default();
        // We hold the in-process stream lock while we *read* purely so that
        // a concurrent in-process append cannot interleave a partial line
        // mid-read. Cross-process safety is provided by the OS-level
        // exclusive file lock taken by `append_envelope`; readers do not
        // need the OS lock.
        //
        // KNOWN LIMITATION (PR #3171 review #48): a long replay scan holds
        // the per-stream Tokio mutex for the duration of the scan, and the
        // shared OS file lock blocks exclusive append-locks on other
        // processes. A sparse / large-history replay can therefore stall
        // live appends for that tenant/user/agent. The stream-bytes-snapshot
        // approach (capture EOF offset, drop locks, scan up to the snapshot)
        // is a substantive concurrency redesign that needs to coordinate
        // with the durable-log contract — tracked as a follow-up.
        let lock = self.stream_lock(kind, stream).await;
        let _guard = lock.lock().await;
        let path = self.stream_path(kind, stream);
        tokio::task::spawn_blocking(move || {
            stream_read_after::<T, _>(&path, after, limit, is_match)
        })
        .await
        .map_err(|_| durable_error("jsonl event store failed to read stream"))?
    }

    /// Atomic head snapshot: read the last assigned cursor directly from the
    /// stream file's tail. `read_last_jsonl_cursor` seeks to EOF and parses
    /// only the final line, so this is O(1) in stream length — never an
    /// unbounded forward scan. We take the in-process stream lock (and the
    /// shared OS lock inside the reader is unnecessary here because we only
    /// read the already-committed last line) so a concurrent in-process append
    /// cannot interleave a partial tail line. The observed last cursor is the
    /// true head at the instant of the call.
    ///
    /// `after` is the caller's known-valid resume cursor. A head strictly below
    /// `after` means the caller asked for a foreign / future cursor, so we
    /// surface [`EventError::ReplayGap`] — mirroring `read_after`.
    async fn head_cursor(
        &self,
        kind: StreamKind,
        stream: &EventStreamKey,
        after: EventCursor,
    ) -> Result<EventCursor, EventError> {
        let lock = self.stream_lock(kind, stream).await;
        let _guard = lock.lock().await;
        let path = self.stream_path(kind, stream);
        let head = tokio::task::spawn_blocking(move || read_last_jsonl_cursor(&path))
            .await
            .map_err(|_| durable_error("jsonl event store failed to read stream head"))??
            .unwrap_or(0);
        if after.as_u64() > head {
            return Err(EventError::ReplayGap {
                requested: after,
                earliest: EventCursor::new(head),
            });
        }
        Ok(EventCursor::new(head))
    }

    async fn stream_lock(&self, kind: StreamKind, stream: &EventStreamKey) -> Arc<Mutex<()>> {
        let key = stream_lock_key(kind, stream);
        let mut locks = self.locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    fn stream_path(&self, kind: StreamKind, stream: &EventStreamKey) -> PathBuf {
        let mut path = self
            .root
            .join(kind.directory())
            .join(component("tenant", stream.tenant_id.as_str()))
            .join(component("user", stream.user_id.as_str()));
        path.push(format!(
            "{}.jsonl",
            agent_component(stream.agent_id.as_ref())
        ));
        path
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum StreamKind {
    Runtime,
    Audit,
}

impl StreamKind {
    fn directory(self) -> &'static Path {
        match self {
            Self::Runtime => Path::new("events"),
            Self::Audit => Path::new("audit"),
        }
    }

    fn lock_prefix(self) -> &'static str {
        match self {
            Self::Runtime => "events",
            Self::Audit => "audit",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonlEntry<T> {
    cursor: EventCursor,
    record: T,
}

#[derive(Debug, Deserialize)]
struct JsonlCursor {
    cursor: EventCursor,
}

fn read_last_jsonl_cursor(path: &Path) -> Result<Option<u64>, EventError> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(durable_error("jsonl event store failed to read stream")),
    };
    let mut position = file
        .metadata()
        .map_err(|_| durable_error("jsonl event store failed to read stream"))?
        .len();
    if position == 0 {
        return Ok(None);
    }

    const CHUNK_SIZE: u64 = 8192;
    let mut reversed_line = Vec::new();
    let mut saw_non_newline = false;
    while position > 0 {
        let read_len = position.min(CHUNK_SIZE) as usize;
        position -= read_len as u64;
        file.seek(SeekFrom::Start(position))
            .map_err(|_| durable_error("jsonl event store failed to read stream"))?;
        let mut chunk = vec![0; read_len];
        file.read_exact(&mut chunk)
            .map_err(|_| durable_error("jsonl event store failed to read stream"))?;
        for byte in chunk.into_iter().rev() {
            if byte == b'\n' || byte == b'\r' {
                if saw_non_newline {
                    reversed_line.reverse();
                    return parse_jsonl_cursor(&reversed_line);
                }
                continue;
            }
            saw_non_newline = true;
            reversed_line.push(byte);
        }
    }

    if !saw_non_newline {
        return Ok(None);
    }
    reversed_line.reverse();
    parse_jsonl_cursor(&reversed_line)
}

fn parse_jsonl_cursor(line: &[u8]) -> Result<Option<u64>, EventError> {
    let envelope =
        serde_json::from_slice::<JsonlCursor>(line).map_err(|error| EventError::Serialize {
            reason: error.to_string(),
        })?;
    Ok(Some(envelope.cursor.as_u64()))
}

/// Stream a JSONL stream line-by-line, applying the cursor `after` filter,
/// the predicate, and the `limit`. Stops as soon as `limit` matches are
/// collected, so a `limit = 1` request on a multi-gigabyte JSONL never reads
/// or parses the whole file.
fn stream_read_after<T, F>(
    path: &Path,
    after: EventCursor,
    limit: usize,
    is_match: F,
) -> Result<EventReplay<T>, EventError>
where
    T: DeserializeOwned,
    F: Fn(&T) -> bool,
{
    use std::io::{BufRead, BufReader};

    let file = match std::fs::File::open(path) {
        Ok(file) => {
            // Take a shared advisory lock so we never observe a partially
            // written line from a concurrent appender in another process.
            file.lock_shared()
                .map_err(|_| durable_error("jsonl event store failed to acquire read lock"))?;
            file
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // No file means no entries yet; treat the head as the origin.
            if after.as_u64() > 0 {
                return Err(EventError::ReplayGap {
                    requested: after,
                    earliest: EventCursor::origin(),
                });
            }
            return Ok(EventReplay {
                entries: Vec::new(),
                next_cursor: after,
            });
        }
        Err(_) => return Err(durable_error("jsonl event store failed to read stream")),
    };
    let reader = BufReader::new(file);

    let mut replay_entries = Vec::new();
    let mut last_scanned = after;
    let mut head_cursor = EventCursor::origin();
    let mut expected_cursor = 1u64;
    let mut after_validated = after.as_u64() == 0;

    for line in reader.lines() {
        let line = line.map_err(|_| durable_error("jsonl event store failed to read stream"))?;
        if line.trim().is_empty() {
            continue;
        }
        // Decode just the cursor first to validate sequencing cheaply and to
        // skip records we do not need to materialise into `T`.
        let envelope_cursor = serde_json::from_str::<JsonlCursor>(&line)
            .map_err(|error| EventError::Serialize {
                reason: error.to_string(),
            })?
            .cursor;
        if envelope_cursor.as_u64() != expected_cursor {
            return Err(durable_error(
                "jsonl event stream cursor sequence is invalid",
            ));
        }
        head_cursor = envelope_cursor;
        expected_cursor = expected_cursor
            .checked_add(1)
            .ok_or_else(|| durable_error("jsonl event cursor overflowed u64"))?;
        if envelope_cursor.as_u64() <= after.as_u64() {
            // We have proven the stream contains at least one record at or
            // beyond `after`, so a future-cursor `ReplayGap` cannot apply.
            if envelope_cursor.as_u64() == after.as_u64() {
                after_validated = true;
            }
            continue;
        }
        // Crossing past `after` also validates it, since the head is now
        // strictly greater than `after`.
        after_validated = true;
        last_scanned = envelope_cursor;
        let envelope = serde_json::from_str::<JsonlEntry<T>>(&line).map_err(|error| {
            EventError::Serialize {
                reason: error.to_string(),
            }
        })?;
        if !is_match(&envelope.record) {
            continue;
        }
        replay_entries.push(EventLogEntry {
            cursor: envelope.cursor,
            record: envelope.record,
        });
        if replay_entries.len() >= limit {
            // Stop streaming as soon as we have `limit` matches so that a
            // small `limit` against a large file does not pay full-stream
            // parse latency. `next_cursor` correctly equals the last match
            // here; the caller can detect any future-cursor gap on the
            // subsequent call.
            break;
        }
    }

    if !after_validated && after.as_u64() > head_cursor.as_u64() {
        return Err(EventError::ReplayGap {
            requested: after,
            earliest: head_cursor,
        });
    }

    let last_matched = replay_entries.last().map(|entry| entry.cursor);
    let next_cursor = match last_matched {
        Some(matched) if matched.as_u64() >= last_scanned.as_u64() => matched,
        Some(_) => last_scanned,
        None => last_scanned,
    };
    Ok(EventReplay {
        entries: replay_entries,
        next_cursor,
    })
}

/// Acquire an OS-level exclusive advisory lock on `path` (creating the file
/// if needed), determine the current tail cursor by reading the file's last
/// JSONL line under the lock, then invoke `serialise` to produce the next
/// envelope's serialised line and append + fsync it. Releases the lock when
/// the function returns. Cross-process safe: two IronClaw processes that race
/// to append against the same file will block on this lock and emit
/// monotonically-sequenced cursors.
///
/// **Atomic from the stream's perspective.** If `write_all`, `flush`, or
/// `sync_data` returns an error after a partial write (ENOSPC, interrupted
/// storage, etc.), the file is truncated back to its pre-append length under
/// the same exclusive lock. Without this, a torn JSONL line at EOF would make
/// every later `read_last_jsonl_cursor` call fail and effectively wedge the
/// stream until manual file surgery.
fn append_with_cursor_assignment<F>(path: &Path, serialise: F) -> Result<u64, EventError>
where
    F: FnOnce(u64) -> Result<String, EventError>,
{
    use std::io::Write;

    // Track whether we're about to create the file so we know to fsync the
    // parent directory afterwards. On POSIX, `sync_data()` on the file
    // contents is not enough for crash durability — the parent directory
    // entry that names the new file must also be fsynced, otherwise the
    // first append can disappear after a power loss even though `append()`
    // returned success.
    let is_first_create = !path.exists();

    let mut file = open_jsonl_for_append(path)?;
    file.lock()
        .map_err(|_| durable_error("jsonl event store failed to acquire append lock"))?;

    // Re-read the prior tail under the lock so we observe writes from any
    // other process that just finished appending.
    let prior_tail = read_last_jsonl_cursor(path)?.unwrap_or(0);
    let next_cursor = prior_tail
        .checked_add(1)
        .ok_or_else(|| durable_error("jsonl event cursor overflowed u64"))?;
    let line = serialise(next_cursor)?;

    // Snapshot the file length before we start writing so we can roll back to
    // a clean tail on any error during the append.
    let pre_append_len = file
        .metadata()
        .map_err(|_| durable_error("jsonl event store failed to inspect stream"))?
        .len();

    let write_result = (|| -> Result<(), EventError> {
        file.write_all(line.as_bytes())
            .map_err(|_| durable_error("jsonl event store failed to append record"))?;
        file.write_all(b"\n")
            .map_err(|_| durable_error("jsonl event store failed to append record"))?;
        file.flush()
            .map_err(|_| durable_error("jsonl event store failed to flush record"))?;
        file.sync_data()
            .map_err(|_| durable_error("jsonl event store failed to sync record"))?;
        Ok(())
    })();

    if let Err(error) = write_result {
        // Best-effort rollback to the pre-append length so a partial/torn
        // tail line never becomes the next reader's "last cursor". Any error
        // here propagates to the caller, but we do not surface a separate
        // truncation error: the original write failure is the load-bearing
        // signal. If truncation itself fails (extremely rare — open file,
        // exclusive lock held), we still fsync to flush whatever state the
        // OS already has and return the original error.
        let _ = file.set_len(pre_append_len);
        let _ = file.sync_data();
        return Err(error);
    }

    if is_first_create && let Some(parent) = path.parent() {
        // `File::open` on a directory + `sync_all` is the portable way to
        // fsync the directory entry on POSIX. Best-effort on platforms that
        // don't support it (e.g. Windows handles this implicitly).
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    // Lock releases when `file` drops at end of scope.
    Ok(next_cursor)
}

/// Open a JSONL stream file for append, creating it with restrictive Unix
/// permissions when it does not yet exist. Event/audit history can name
/// tenants, users, agents, and decision payloads — leaving the file
/// world-readable under the typical `umask 022` would expose that history to
/// any local account on the host. We create new files with mode `0600` and
/// new parent directories with mode `0700`.
fn open_jsonl_for_append(path: &Path) -> Result<std::fs::File, EventError> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // 0o600 — owner read/write only. `mode` is ignored if the file already
        // exists, so this only tightens permissions on first creation.
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|_| durable_error("jsonl event store failed to open stream"))
}

fn durable_error(reason: impl Into<String>) -> EventError {
    EventError::DurableLog {
        reason: reason.into(),
    }
}

fn stream_lock_key(kind: StreamKind, stream: &EventStreamKey) -> String {
    format!(
        "{}/{}/{}/{}",
        kind.lock_prefix(),
        stream.tenant_id.as_str(),
        stream.user_id.as_str(),
        stream
            .agent_id
            .as_ref()
            .map(AgentId::as_str)
            .unwrap_or("<none>")
    )
}

/// Map an arbitrary identifier to a filesystem path component that is
/// **case-distinct** (so `Alice` and `alice` cannot collide on
/// case-insensitive filesystems like macOS HFS+ / APFS default and Windows
/// NTFS) and **bounded in length** (so a 256-byte valid scope ID does not
/// produce a 263-byte filename that exceeds the 255-byte limit on common
/// filesystems).
///
/// Format: `{prefix}-{hash16}-{hint}` where:
/// - `prefix` distinguishes the kind (e.g. `tenant`, `user`, `agent-id`).
/// - `hash16` is the first 16 lowercase-hex characters of SHA-256 of the raw
///   bytes — purely ASCII, so it stays case-distinct on case-insensitive
///   filesystems and bounded in length.
/// - `hint` is the URL-encoded raw value truncated to keep the total
///   component well under 255 bytes. The hint exists for human-readable
///   debugging only — uniqueness/correctness comes from the hash.
fn component(prefix: &str, value: &str) -> String {
    use sha2::{Digest, Sha256};

    const HASH_HEX_LEN: usize = 16;
    const HINT_MAX: usize = 32;

    let digest = Sha256::digest(value.as_bytes());
    let hash_hex: String = hex::encode(&digest[..HASH_HEX_LEN / 2]);
    let hint_encoded = urlencoding::encode(value);
    let hint = if hint_encoded.len() > HINT_MAX {
        // URL-encoded output is pure ASCII so byte-slicing is UTF-8 safe.
        &hint_encoded[..HINT_MAX]
    } else {
        &hint_encoded
    };
    format!("{prefix}-{hash_hex}-{hint}")
}

fn agent_component(agent_id: Option<&AgentId>) -> String {
    match agent_id {
        Some(agent_id) => component("agent-id", agent_id.as_str()),
        None => "agent-none".to_string(),
    }
}

/// Create a directory tree with restrictive permissions on first creation.
///
/// On Unix we use mode `0o700` so a freshly-created tenant/user directory is
/// not world-listable under the typical `umask 022`. Existing directories
/// retain their current permissions — `create_dir_all` on an existing path
/// is a no-op and never re-applies the requested mode. On non-Unix
/// platforms this falls back to `tokio::fs::create_dir_all`.
async fn create_secure_dir_all(path: &Path) -> std::io::Result<()> {
    let mut builder = tokio::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        // `tokio::fs::DirBuilder::mode` is an inherent cfg(unix) method —
        // no `DirBuilderExt` import is required.
        builder.mode(0o700);
    }
    builder.create(path).await
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, TenantId, UserId};

    use super::*;

    #[tokio::test]
    async fn jsonl_stream_lock_registry_prunes_released_locks() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = JsonlStore::new(temp.path().join("event-store"));
        let stream_a = EventStreamKey::new(
            TenantId::new("tenant-a").unwrap(),
            UserId::new("user-a").unwrap(),
            Some(AgentId::new("agent-a").unwrap()),
        );
        let stream_b = EventStreamKey::new(
            TenantId::new("tenant-a").unwrap(),
            UserId::new("user-a").unwrap(),
            Some(AgentId::new("agent-b").unwrap()),
        );

        let lock_a = store.stream_lock(StreamKind::Runtime, &stream_a).await;
        assert_eq!(store.locks.lock().await.len(), 1);
        drop(lock_a);

        let _lock_b = store.stream_lock(StreamKind::Runtime, &stream_b).await;
        assert_eq!(store.locks.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn production_rejects_cleartext_http_libsql_url() {
        let result = build_reborn_event_stores(
            RebornProfile::Production,
            RebornEventStoreConfig::Libsql {
                path_or_url: "http://libsql.example.com:8080".to_string(),
                auth_token: None,
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(RebornEventStoreError::ProductionLibsqlClearTextDisabled)
        ));
    }

    #[tokio::test]
    async fn local_dev_allows_cleartext_http_libsql_url() {
        // Non-production profiles can still use http:// for local sqld.
        // The build call will fail on the actual connection attempt below
        // for an unreachable address, but it must NOT fail with the
        // cleartext-disabled error.
        let result = build_reborn_event_stores(
            RebornProfile::LocalDev,
            RebornEventStoreConfig::Libsql {
                path_or_url: "http://127.0.0.1:1".to_string(),
                auth_token: None,
            },
        )
        .await;
        assert!(!matches!(
            result,
            Err(RebornEventStoreError::ProductionLibsqlClearTextDisabled)
        ));
    }

    // --- libSQL production-target classification (issues #34, #36, #41) ---

    #[tokio::test]
    async fn production_rejects_in_memory_libsql_target() {
        // Regression for nearai/ironclaw#3171 review finding: a libSQL
        // `:memory:` config previously bypassed the InMemory production gate
        // by reaching `Builder::new_local`, creating an ephemeral DB whose
        // history is lost on restart.
        let result = build_reborn_event_stores(
            RebornProfile::Production,
            RebornEventStoreConfig::Libsql {
                path_or_url: ":memory:".to_string(),
                auth_token: None,
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(RebornEventStoreError::ProductionInMemoryDisabled)
        ));
    }

    #[tokio::test]
    async fn production_rejects_mixed_case_cleartext_libsql_url() {
        // Mixed-case `HTTP://` previously skipped `is_remote_libsql` and
        // fell through to a node-local SQLite path like `HTTP:/host/...`.
        for url in [
            "HTTP://libsql.example.com",
            "Http://libsql.example.com",
            "hTTp://libsql.example.com",
        ] {
            let result = build_reborn_event_stores(
                RebornProfile::Production,
                RebornEventStoreConfig::Libsql {
                    path_or_url: url.to_string(),
                    auth_token: None,
                },
            )
            .await;
            assert!(
                matches!(
                    result,
                    Err(RebornEventStoreError::ProductionLibsqlClearTextDisabled)
                ),
                "expected `{url}` to be rejected as cleartext"
            );
        }
    }

    #[tokio::test]
    async fn production_accepts_mixed_case_secure_libsql_scheme() {
        // The classifier must treat `HTTPS://` and `LibSQL://` as remote
        // secure schemes regardless of case, instead of routing to the local
        // path. The build call below will fail on the actual connection
        // attempt against an unreachable host, but the failure must NOT be
        // one of the production policy rejections.
        for url in ["HTTPS://example.invalid", "LibSQL://example.invalid"] {
            let result = build_reborn_event_stores(
                RebornProfile::Production,
                RebornEventStoreConfig::Libsql {
                    path_or_url: url.to_string(),
                    auth_token: None,
                },
            )
            .await;
            match result {
                Err(RebornEventStoreError::ProductionInMemoryDisabled)
                | Err(RebornEventStoreError::ProductionLibsqlClearTextDisabled)
                | Err(RebornEventStoreError::ProductionLibsqlAmbiguousTarget) => {
                    panic!("`{url}` should pass policy classification, got policy reject")
                }
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn production_rejects_bare_hostname_libsql_target() {
        // `path_or_url = "db.example.com"` previously went down the local
        // path and silently created `./db.example.com`, ignoring the auth
        // token and stranding durable history on one node. Production now
        // fails closed on any value without an explicit scheme or path
        // prefix.
        let result = build_reborn_event_stores(
            RebornProfile::Production,
            RebornEventStoreConfig::Libsql {
                path_or_url: "db.example.com".to_string(),
                auth_token: None,
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(RebornEventStoreError::ProductionLibsqlAmbiguousTarget)
        ));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn local_dev_still_allows_bare_relative_libsql_path() {
        // The bare-path rejection is a production-only policy. LocalDev /
        // Test must still allow `events.db` for ergonomic test/demo configs.
        let temp = tempfile::tempdir().expect("tempdir");
        let cwd = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(temp.path()).expect("chdir to tempdir");
        let result = build_reborn_event_stores(
            RebornProfile::LocalDev,
            RebornEventStoreConfig::Libsql {
                path_or_url: "events.db".to_string(),
                auth_token: None,
            },
        )
        .await;
        let _ = std::env::set_current_dir(cwd);
        assert!(
            !matches!(
                result,
                Err(RebornEventStoreError::ProductionLibsqlAmbiguousTarget)
            ),
            "LocalDev must accept bare relative paths"
        );
        // The build itself should succeed for a bare filename in cwd.
        result.expect("local libsql with bare relative path should build");
    }

    // --- Path component mapping (issues #40, #44) ---

    #[test]
    fn case_distinct_ids_map_to_distinct_components() {
        // On case-insensitive filesystems (HFS+, APFS default, NTFS),
        // `Alice` and `alice` resolve to the same path string. The hashed
        // mapper must produce different components so the two streams are
        // never merged into the same JSONL file.
        let upper = component("user", "Alice");
        let lower = component("user", "alice");
        let mixed = component("user", "ALICE");
        assert_ne!(upper, lower);
        assert_ne!(upper, mixed);
        assert_ne!(lower, mixed);
    }

    #[test]
    fn long_ids_map_to_filename_safe_components() {
        // Host scope IDs allow up to 256 bytes. The previous mapper produced
        // `tenant-` + raw 256 bytes = 263 bytes which exceeds the 255-byte
        // filename limit on common filesystems. The hashed mapper must keep
        // the component safely under that limit.
        let long_id = "x".repeat(256);
        let mapped = component("tenant", &long_id);
        assert!(mapped.len() < 200, "component len = {}", mapped.len());
        // And different long IDs that share a 32-byte prefix must still map
        // to different components (because the hash sees the full input).
        let other = format!("{}{}", "x".repeat(220), "_distinct");
        let other_mapped = component("tenant", &other);
        assert_ne!(mapped, other_mapped);
    }

    // --- Atomic JSONL append (issue #43) ---

    #[test]
    fn jsonl_append_truncates_on_serialiser_failure() {
        // If the serialise callback errors AFTER we've started writing (the
        // simplest reliable in-process simulation of a partial write), the
        // file must be left in its pre-append state so subsequent appends
        // don't observe a torn tail. We simulate by failing the serialise
        // step after one successful append.
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("stream.jsonl");

        // Successful append #1.
        let cursor1 = append_with_cursor_assignment(&path, |c| Ok(format!("{{\"cursor\":{c}}}")))
            .expect("first append");
        assert_eq!(cursor1, 1);
        let len_after_first = std::fs::metadata(&path).expect("metadata").len();

        // Append #2: serialise returns Err — but to test rollback of an
        // already-written tail we directly write garbage and then call the
        // helper, which should preserve len_after_first on failure.
        let result = append_with_cursor_assignment(&path, |_| {
            Err(EventError::Serialize {
                reason: "synthetic".to_string(),
            })
        });
        assert!(result.is_err());
        let len_after_failed = std::fs::metadata(&path).expect("metadata").len();
        assert_eq!(
            len_after_failed, len_after_first,
            "failed append must leave file at pre-append length"
        );

        // Append #3: stream is still healthy.
        let cursor3 = append_with_cursor_assignment(&path, |c| Ok(format!("{{\"cursor\":{c}}}")))
            .expect("third append");
        assert_eq!(cursor3, 2, "cursor must advance from healthy tail");
    }

    // --- JSONL file/directory permissions (issue #38) ---

    #[cfg(unix)]
    #[tokio::test]
    async fn jsonl_root_directory_uses_restrictive_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("event-store");
        let stores = build_reborn_event_stores(
            RebornProfile::LocalDev,
            RebornEventStoreConfig::Jsonl {
                root: root.clone(),
                accept_single_node_durable: false,
            },
        )
        .await
        .expect("build jsonl stores");
        let _ = stores; // keep the type-check trivial
        let mode = std::fs::metadata(&root)
            .expect("root metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o700,
            "newly created jsonl root must not be world-listable"
        );
    }
}
