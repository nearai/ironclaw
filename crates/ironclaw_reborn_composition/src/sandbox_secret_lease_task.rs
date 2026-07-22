//! Composition-owned resolver + per-user socket daemon for the sandboxed
//! shell profile's secret-lease protocol (Phase C, Task 5).
//!
//! Mirrors `sandbox_egress_proxy_task.rs`'s split exactly: the lease
//! protocol/server core (`ironclaw_host_runtime::sandbox_process::secret_lease`)
//! is deliberately secrets-store-agnostic — this module connects it to the
//! real `ironclaw_secrets::SecretStore` the runtime already builds, binds
//! one Unix socket per user, and owns its cancellation via
//! `SandboxSecretLeaseDaemonHandle` (declared canonically in
//! `sandbox_composition.rs`, Task A0).
//!
//! **Per-user socket from the start:** the design forbids an interim
//! tenant-level socket — production binds at
//! `sockets_root/users/<user-digest>/broker.sock`, keyed via
//! `RebornSandboxUserKey::from_tenant_user`, from day one.
//!
//! **Known OAuth gap (intentional Phase C scope, not an oversight):**
//! [`CompositionSandboxSecretLeaseResolver`] resolves every lease request
//! directly against `SecretStore`, bypassing
//! `RuntimeCredentialAccountResolver` entirely. OAuth-managed secrets need
//! `provider`/`setup`/`provider_scopes` context that
//! `RuntimeCredentialAccountRequest` carries but the wire protocol's
//! `secret_name: String` has no slot for — see the doc comment on the
//! resolver below.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_api::{ResourceScope, SecretHandle};
use ironclaw_host_runtime::{
    RebornSandboxUserKey, SandboxSecretLease, SandboxSecretLeaseError, SandboxSecretLeaseResolver,
    SandboxSecretLeaseServer,
};
use ironclaw_secrets::SecretStore;
use tokio::sync::watch;

use crate::RebornBuildError;
use crate::sandbox_composition::SandboxSecretLeaseDaemonHandle;

/// TTL communicated to the container-side `ironclaw-lease` shim for every
/// leased secret. The daemon never caches or replays material (a fresh
/// `lease_once`/`consume` pair is minted on every call), so this is
/// advisory metadata for the caller, not a cache lifetime here.
const DEFAULT_SECRET_LEASE_TTL_SECONDS: u64 = 300;

/// How long [`spawn_sandbox_secret_lease_socket`] waits for the spawned
/// accept loop's initial bind to become observable (the socket file to
/// appear) before treating the spawn as failed. `bind_and_serve` performs
/// the `UnixListener::bind` inside the spawned task itself (unlike the
/// egress proxy, which binds synchronously before spawning), so this is
/// the seam that turns an unbindable socket into a build-time error
/// instead of a daemon that silently never came up.
const SOCKET_BIND_POLL_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolves a lease request's `secret_name` (opaque wire string, e.g.
/// `"sample_secret"`) to a `SecretHandle` and round-trips it through
/// `SecretStore::lease_once`/`consume`. A fresh `lease_once`/`consume` pair
/// is minted on EVERY call — this is pinned production behavior, not an
/// implementation detail: the lease daemon never caches or replays
/// material, so each `ironclaw-lease <name>` invocation inside a container
/// gets its own one-shot lease from the store. See
/// `resolver_leases_fresh_material_on_each_call`.
///
/// Follow-up (file issue at PR): extend wire protocol with provider/setup/scopes
/// for OAuth-managed secrets; direct SecretStore path only here.
pub(crate) struct CompositionSandboxSecretLeaseResolver {
    secrets: Arc<dyn SecretStore>,
}

impl CompositionSandboxSecretLeaseResolver {
    pub(crate) fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self { secrets }
    }
}

#[async_trait::async_trait]
impl SandboxSecretLeaseResolver for CompositionSandboxSecretLeaseResolver {
    async fn resolve_lease(
        &self,
        scope: &ResourceScope,
        secret_name: &str,
    ) -> Result<SandboxSecretLease, SandboxSecretLeaseError> {
        // The store's own error text is never inspected or forwarded here —
        // only mapped to the wire-safe, material-free `SandboxSecretLeaseError`
        // variants. See the module doc on `secret_lease.rs` for why that type
        // makes redaction a property of the type rather than a discipline.
        let handle =
            SecretHandle::new(secret_name).map_err(|_| SandboxSecretLeaseError::UnknownSecret)?;
        let lease = self
            .secrets
            .lease_once(scope, &handle)
            .await
            .map_err(|_| SandboxSecretLeaseError::UnknownSecret)?;
        let material = self
            .secrets
            .consume(scope, lease.id)
            .await
            .map_err(|_| SandboxSecretLeaseError::BackendUnavailable)?;
        Ok(SandboxSecretLease {
            material,
            ttl_seconds: Some(DEFAULT_SECRET_LEASE_TTL_SECONDS),
        })
    }
}

/// Binds one lease socket at `sockets_root/users/<user-digest>/broker.sock`
/// for `user_key` and spawns its accept loop. This IS the production key
/// from day one — no interim tenant-only socket stage.
///
/// Fails closed (mirrors `spawn_sandbox_egress_proxy`, unlike the
/// best-effort reaper): a secret-lease daemon that never bound would mean
/// every `ironclaw-lease` invocation inside the sandboxed container hangs
/// or fails opaquely instead of composition surfacing the problem at boot.
pub(crate) async fn spawn_sandbox_secret_lease_socket(
    resolver: Arc<dyn SandboxSecretLeaseResolver>,
    scope: ResourceScope,
    user_key: RebornSandboxUserKey,
    sockets_root: &Path,
) -> Result<SandboxSecretLeaseDaemonHandle, RebornBuildError> {
    // `socket_path` (not `workspace_path`) — see its doc comment on
    // `RebornSandboxUserKey`: a Unix socket path is capped at 104
    // (macOS) / 108 (Linux) bytes, far tighter than `workspace_path`'s
    // full 64-hex-char digest tolerates once nested under a real root.
    let socket_path = user_key.socket_path(sockets_root);
    let socket_dir = socket_path
        .parent()
        .expect("socket_path always has a users/<digest> parent")
        .to_path_buf();
    std::fs::create_dir_all(&socket_dir).map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!(
            "sandbox secret lease socket directory {socket_dir:?} could not be created: {error}"
        ),
    })?;
    // A stale socket file from a prior process (e.g. an unclean shutdown)
    // would otherwise make `UnixListener::bind` fail with "address in use".
    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }

    let server = SandboxSecretLeaseServer::new(resolver, scope);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let socket_path_for_serve = socket_path.clone();
    let handle = tokio::spawn(async move {
        if let Err(error) = server
            .bind_and_serve(&socket_path_for_serve, shutdown_rx)
            .await
        {
            tracing::debug!(?error, "sandbox secret lease daemon exited");
        }
    });

    if !wait_for_socket(&socket_path, SOCKET_BIND_POLL_TIMEOUT).await {
        handle.abort();
        return Err(RebornBuildError::InvalidConfig {
            reason: format!(
                "sandbox secret lease daemon did not bind {socket_path:?} within {SOCKET_BIND_POLL_TIMEOUT:?}"
            ),
        });
    }

    Ok(SandboxSecretLeaseDaemonHandle::new(
        shutdown_tx,
        handle,
        socket_path,
    ))
}

/// Polls for `socket_path` to appear, so callers never race the spawned
/// `bind_and_serve` task's initial bind. Returns `false` on timeout.
async fn wait_for_socket(socket_path: &Path, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if socket_path.exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    socket_path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, InvocationId, ProjectId, TenantId, ThreadId, UserId};
    use ironclaw_secrets::{
        SecretLease, SecretLeaseId, SecretLeaseStatus, SecretMaterial, SecretMetadata,
        SecretStoreError,
    };
    use secrecy::ExposeSecret;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    const TEST_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

    /// A tempdir rooted at `/tmp` rather than `std::env::temp_dir()`.
    /// `TMPDIR` on macOS resolves to a deep, per-process, randomized path
    /// (`/var/folders/xx/<hash>/T/`) that alone consumes most of a Unix
    /// socket's ~104-byte `sun_path` budget before this module ever adds
    /// its own `users/<digest>/broker.sock` suffix — using the flat `/tmp`
    /// (symlinked to `/private/tmp` on macOS) keeps these tests portable
    /// across the same OS constraint `RebornSandboxUserKey::socket_path`
    /// exists to respect.
    fn short_tempdir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("ic-lease-")
            .tempdir_in("/tmp")
            .expect("short tempdir under /tmp")
    }

    fn scope(tenant: &str, user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).expect("valid tenant id"),
            user_id: UserId::new(user).expect("valid user id"),
            agent_id: None::<AgentId>,
            project_id: None::<ProjectId>,
            mission_id: None,
            thread_id: None::<ThreadId>,
            invocation_id: InvocationId::new(),
        }
    }

    /// Local test double against the real 8-method `SecretStore` trait —
    /// `ironclaw_secrets` has no `test-support` feature and no in-memory
    /// store today. The resolver only calls `lease_once`/`consume` (and
    /// `put` seeds the fixture); the other 5 methods are `unreachable!`
    /// with a stated reason, the same convention Task A0's
    /// `AlwaysAbsentRunStateStore` uses.
    ///
    /// Stored as `Vec`s with linear scan (not `HashMap`) — `ResourceScope`
    /// implements `PartialEq`/`Eq` but not `Hash`.
    struct FakeSecretStore {
        secrets: tokio::sync::Mutex<Vec<(ResourceScope, SecretHandle, SecretMaterial)>>,
        leases: tokio::sync::Mutex<Vec<(SecretLeaseId, ResourceScope, SecretHandle)>>,
    }

    impl FakeSecretStore {
        fn new() -> Self {
            Self {
                secrets: tokio::sync::Mutex::new(Vec::new()),
                leases: tokio::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl SecretStore for FakeSecretStore {
        async fn put(
            &self,
            scope: ResourceScope,
            handle: SecretHandle,
            material: SecretMaterial,
            expires_at: Option<ironclaw_host_api::Timestamp>,
        ) -> Result<SecretMetadata, SecretStoreError> {
            self.secrets
                .lock()
                .await
                .push((scope.clone(), handle.clone(), material));
            Ok(SecretMetadata {
                scope,
                handle,
                expires_at,
            })
        }

        async fn lease_once(
            &self,
            scope: &ResourceScope,
            handle: &SecretHandle,
        ) -> Result<SecretLease, SecretStoreError> {
            let known = self
                .secrets
                .lock()
                .await
                .iter()
                .any(|(s, h, _)| s == scope && h == handle);
            if !known {
                return Err(SecretStoreError::UnknownSecret {
                    scope: Box::new(scope.clone()),
                    handle: handle.clone(),
                });
            }
            let id = SecretLeaseId::new();
            self.leases
                .lock()
                .await
                .push((id, scope.clone(), handle.clone()));
            Ok(SecretLease {
                id,
                scope: scope.clone(),
                handle: handle.clone(),
                status: SecretLeaseStatus::Active,
            })
        }

        async fn consume(
            &self,
            scope: &ResourceScope,
            lease_id: SecretLeaseId,
        ) -> Result<SecretMaterial, SecretStoreError> {
            let (lease_scope, handle) = {
                let mut leases = self.leases.lock().await;
                let position = leases.iter().position(|(id, _, _)| *id == lease_id).ok_or(
                    SecretStoreError::UnknownLease {
                        scope: Box::new(scope.clone()),
                        lease_id,
                    },
                )?;
                let (_, lease_scope, handle) = leases.remove(position);
                (lease_scope, handle)
            };
            self.secrets
                .lock()
                .await
                .iter()
                .find(|(s, h, _)| *s == lease_scope && *h == handle)
                .map(|(_, _, material)| material.clone())
                .ok_or(SecretStoreError::UnknownSecret {
                    scope: Box::new(scope.clone()),
                    handle,
                })
        }

        // Not exercised by CompositionSandboxSecretLeaseResolver — same
        // convention as Task A0's AlwaysAbsentRunStateStore. If a future
        // test calls one, make it real then.
        async fn metadata(
            &self,
            _scope: &ResourceScope,
            _handle: &SecretHandle,
        ) -> Result<Option<SecretMetadata>, SecretStoreError> {
            unreachable!("FakeSecretStore::metadata not exercised")
        }
        async fn metadata_for_scope(
            &self,
            _scope: &ResourceScope,
        ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
            unreachable!("FakeSecretStore::metadata_for_scope not exercised")
        }
        async fn delete(
            &self,
            _scope: &ResourceScope,
            _handle: &SecretHandle,
        ) -> Result<bool, SecretStoreError> {
            unreachable!("FakeSecretStore::delete not exercised")
        }
        async fn revoke(
            &self,
            _scope: &ResourceScope,
            _lease_id: SecretLeaseId,
        ) -> Result<SecretLease, SecretStoreError> {
            unreachable!("FakeSecretStore::revoke not exercised")
        }
        async fn leases_for_scope(
            &self,
            _scope: &ResourceScope,
        ) -> Result<Vec<SecretLease>, SecretStoreError> {
            unreachable!("FakeSecretStore::leases_for_scope not exercised")
        }
    }

    #[tokio::test]
    async fn resolver_leases_and_consumes_a_known_handle() {
        let store: Arc<dyn SecretStore> = Arc::new(FakeSecretStore::new());
        let scope = scope("tenant-a", "user-a");
        let handle = SecretHandle::new("sample_secret").expect("valid handle");
        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("sample_secret_material".to_string()),
                None,
            )
            .await
            .expect("seed secret");

        let resolver = CompositionSandboxSecretLeaseResolver::new(store);
        let lease = resolver
            .resolve_lease(&scope, "sample_secret")
            .await
            .expect("known secret leases successfully");

        assert_eq!(lease.material.expose_secret(), "sample_secret_material");
        assert_eq!(lease.ttl_seconds, Some(DEFAULT_SECRET_LEASE_TTL_SECONDS));
    }

    #[tokio::test]
    async fn resolver_maps_unknown_handle_to_unknown_secret_error() {
        let store: Arc<dyn SecretStore> = Arc::new(FakeSecretStore::new());
        let resolver = CompositionSandboxSecretLeaseResolver::new(store);
        let scope = scope("tenant-a", "user-a");

        let outcome = resolver.resolve_lease(&scope, "nonexistent_secret").await;

        match outcome {
            Ok(_) => panic!("unseeded secret must not lease"),
            Err(error) => assert_eq!(error, SandboxSecretLeaseError::UnknownSecret),
        }
    }

    #[tokio::test]
    async fn resolver_leases_fresh_material_on_each_call() {
        let store: Arc<dyn SecretStore> = Arc::new(FakeSecretStore::new());
        let scope = scope("tenant-a", "user-a");
        let handle = SecretHandle::new("rotating_secret").expect("valid handle");
        store
            .put(
                scope.clone(),
                handle,
                SecretMaterial::from("material-value".to_string()),
                None,
            )
            .await
            .expect("seed secret");

        let resolver = CompositionSandboxSecretLeaseResolver::new(store);
        let first = resolver
            .resolve_lease(&scope, "rotating_secret")
            .await
            .expect("first lease succeeds");
        let second = resolver
            .resolve_lease(&scope, "rotating_secret")
            .await
            .expect("second lease succeeds independently, minting its own lease_once/consume pair");

        assert_eq!(first.material.expose_secret(), "material-value");
        assert_eq!(second.material.expose_secret(), "material-value");
    }

    #[tokio::test]
    async fn spawn_binds_a_reachable_socket_at_the_per_user_path_and_shutdown_stops_it() {
        let dir = short_tempdir();
        let sockets_root = dir.path().to_path_buf();
        let tenant_id = TenantId::new("tenant-a").expect("valid tenant id");
        let user_id = UserId::new("user-a").expect("valid user id");
        let user_key = RebornSandboxUserKey::from_tenant_user(&tenant_id, &user_id);
        let store: Arc<dyn SecretStore> = Arc::new(FakeSecretStore::new());
        let resolver: Arc<dyn SandboxSecretLeaseResolver> =
            Arc::new(CompositionSandboxSecretLeaseResolver::new(store));

        let handle = spawn_sandbox_secret_lease_socket(
            resolver,
            scope("tenant-a", "user-a"),
            user_key.clone(),
            &sockets_root,
        )
        .await
        .expect("binds within an empty tempdir");

        let expected_socket_path = user_key.socket_path(&sockets_root);
        assert_eq!(
            handle.socket_path, expected_socket_path,
            "socket must bind at the per-user path RebornSandboxUserKey computes"
        );
        assert!(
            handle.socket_path.starts_with(sockets_root.join("users")),
            "socket path must be under a per-user directory, not a flat <tenant>.sock: {:?}",
            handle.socket_path
        );

        let connected = UnixStream::connect(&handle.socket_path).await;
        assert!(
            connected.is_ok(),
            "expected to connect to the spawned lease socket at {:?}: {connected:?}",
            handle.socket_path
        );

        handle.shutdown(TEST_SHUTDOWN_TIMEOUT).await;
    }

    #[tokio::test]
    async fn shutdown_stops_a_running_daemon_before_the_timeout() {
        let dir = short_tempdir();
        let sockets_root = dir.path().to_path_buf();
        let tenant_id = TenantId::new("tenant-b").expect("valid tenant id");
        let user_id = UserId::new("user-b").expect("valid user id");
        let user_key = RebornSandboxUserKey::from_tenant_user(&tenant_id, &user_id);
        let store: Arc<dyn SecretStore> = Arc::new(FakeSecretStore::new());
        let resolver: Arc<dyn SandboxSecretLeaseResolver> =
            Arc::new(CompositionSandboxSecretLeaseResolver::new(store));

        let handle = spawn_sandbox_secret_lease_socket(
            resolver,
            scope("tenant-b", "user-b"),
            user_key,
            &sockets_root,
        )
        .await
        .expect("binds within an empty tempdir");

        handle.shutdown(TEST_SHUTDOWN_TIMEOUT).await;
        // Reaching here without hanging proves the shutdown signal reached
        // the task and the join completed inside the timeout.
    }

    async fn raw_lease_request(socket_path: &Path, secret_name: &str) -> serde_json::Value {
        let mut client = UnixStream::connect(socket_path)
            .await
            .expect("client connects to the lease socket");
        client
            .write_all(format!("{{\"secret_name\":\"{secret_name}\"}}\n").as_bytes())
            .await
            .expect("request writes");
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .await
            .expect("reads the full response then EOF");
        serde_json::from_slice(&response).expect("valid json response")
    }

    #[tokio::test]
    async fn spawned_daemon_serves_a_lease_through_the_real_resolver_and_store() {
        let dir = short_tempdir();
        let sockets_root = dir.path().to_path_buf();
        let tenant_id = TenantId::new("tenant-c").expect("valid tenant id");
        let user_id = UserId::new("user-c").expect("valid user id");
        let user_key = RebornSandboxUserKey::from_tenant_user(&tenant_id, &user_id);
        let lease_scope = scope("tenant-c", "user-c");
        let store: Arc<dyn SecretStore> = Arc::new(FakeSecretStore::new());
        store
            .put(
                lease_scope.clone(),
                SecretHandle::new("sample_secret").expect("valid handle"),
                SecretMaterial::from("sample_secret_material_over_the_wire".to_string()),
                None,
            )
            .await
            .expect("seed secret");
        let resolver: Arc<dyn SandboxSecretLeaseResolver> = Arc::new(
            CompositionSandboxSecretLeaseResolver::new(Arc::clone(&store)),
        );

        let handle =
            spawn_sandbox_secret_lease_socket(resolver, lease_scope, user_key, &sockets_root)
                .await
                .expect("binds within an empty tempdir");

        let response = raw_lease_request(&handle.socket_path, "sample_secret").await;
        assert_eq!(response["status"], "ok");
        assert_eq!(response["material"], "sample_secret_material_over_the_wire");

        let missing = raw_lease_request(&handle.socket_path, "absent_secret").await;
        assert_eq!(missing["status"], "error");
        assert_eq!(missing["reason"], "unknown_secret");

        handle.shutdown(TEST_SHUTDOWN_TIMEOUT).await;
    }
}
