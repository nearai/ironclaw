//! Single composition seam for every sandboxed-profile runtime background
//! task and shared handle. `factory.rs` gets exactly one construction call
//! site (`SandboxRuntimeBindings::build`, inside `build_local_runtime`) and
//! `runtime.rs` gets exactly one shutdown call
//! (`SandboxRuntimeBindings::shutdown_all`) for the whole family — the
//! per-user activity registry (Task A2), the reaper handle (already built;
//! Task A5 flips its wiring to use the activity registry), and Phase C's
//! egress-proxy/secret-lease daemon handles all live behind this one
//! struct instead of `factory.rs` growing a new field and a new shutdown
//! block per sandbox subsystem.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_api::{InvocationId, ResourceScope, UserId};
use ironclaw_host_runtime::{RebornSandboxUserKey, SandboxActivityRegistry};
use ironclaw_resources::ResourceGovernor;
use ironclaw_secrets::SecretStore;

use crate::RebornBuildError;
use crate::input::RebornLocalRuntimeIdentity;

/// Owned handle to a spawned [`ironclaw_host_runtime::BoundEgressAllowlistProxy::serve`]
/// task. Declared canonically here (not in `sandbox_egress_proxy_task.rs`)
/// so `SandboxRuntimeBindings`'s shape is stable across Phase A/Phase C —
/// `sandbox_egress_proxy_task::spawn_sandbox_egress_proxy` constructs one via
/// [`SandboxEgressProxyRuntimeHandle::new`] and returns it.
///
/// `pub` (not `pub(crate)`, unlike [`SandboxSecretLeaseDaemonHandle`]):
/// `tenant_sandbox_process_binding` (`sandbox_boot.rs`) spawns the
/// production instance and hands it back on `TenantSandboxBinding` so the
/// assembling binary (`ironclaw_reborn_cli`) can thread it, opaquely, into
/// `RebornBuildInput` for `SandboxRuntimeBindings::build` to take ownership
/// of later — the same round-trip-through-the-binary shape
/// `SandboxActivityRegistry` already uses. Its fields and methods stay
/// `pub(crate)`, so the binary can only move the value along, never
/// construct or drive one itself.
pub struct SandboxEgressProxyRuntimeHandle {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    handle: tokio::task::JoinHandle<()>,
    pub(crate) local_addr: std::net::SocketAddr,
}

impl SandboxEgressProxyRuntimeHandle {
    pub(crate) fn new(
        shutdown_tx: tokio::sync::watch::Sender<bool>,
        handle: tokio::task::JoinHandle<()>,
        local_addr: std::net::SocketAddr,
    ) -> Self {
        Self {
            shutdown_tx,
            handle,
            local_addr,
        }
    }

    /// Signals the proxy's accept loop to stop and awaits the task,
    /// aborting it if it has not stopped within `timeout`. Mirrors
    /// `SandboxReaperRuntimeHandle::shutdown` exactly.
    pub(crate) async fn shutdown(self, timeout: Duration) {
        let _ = self.shutdown_tx.send(true);
        let mut handle = self.handle;
        match tokio::time::timeout(timeout, &mut handle).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::debug!(?error, "sandbox egress proxy task join failed");
            }
            Err(_) => {
                tracing::debug!(
                    ?timeout,
                    "sandbox egress proxy did not stop before shutdown timeout; aborting"
                );
                handle.abort();
                if let Err(error) = handle.await
                    && error.is_panic()
                {
                    tracing::debug!(?error, "aborted sandbox egress proxy task panicked");
                }
            }
        }
    }
}

/// Owned handle to a spawned per-user
/// [`ironclaw_host_runtime::SandboxSecretLeaseServer`]'s accept-loop task
/// (its `bind_and_serve` method). Declared canonically here (not in
/// `sandbox_secret_lease_task.rs`), the
/// same split `SandboxEgressProxyRuntimeHandle` uses —
/// `sandbox_secret_lease_task::spawn_sandbox_secret_lease_socket` constructs
/// one via [`SandboxSecretLeaseDaemonHandle::new`] and returns it.
pub(crate) struct SandboxSecretLeaseDaemonHandle {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    handle: tokio::task::JoinHandle<()>,
    pub(crate) socket_path: std::path::PathBuf,
}

impl SandboxSecretLeaseDaemonHandle {
    pub(crate) fn new(
        shutdown_tx: tokio::sync::watch::Sender<bool>,
        handle: tokio::task::JoinHandle<()>,
        socket_path: std::path::PathBuf,
    ) -> Self {
        Self {
            shutdown_tx,
            handle,
            socket_path,
        }
    }

    /// Signals the daemon's accept loop to stop and awaits the task,
    /// aborting it if it has not stopped within `timeout`. Mirrors
    /// `SandboxEgressProxyRuntimeHandle::shutdown` exactly.
    pub(crate) async fn shutdown(self, timeout: Duration) {
        let _ = self.shutdown_tx.send(true);
        let socket_path = self.socket_path;
        let mut handle = self.handle;
        match tokio::time::timeout(timeout, &mut handle).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::debug!(
                    ?error,
                    ?socket_path,
                    "sandbox secret lease daemon task join failed"
                );
            }
            Err(_) => {
                tracing::debug!(
                    ?timeout,
                    ?socket_path,
                    "sandbox secret lease daemon did not stop before shutdown timeout; aborting"
                );
                handle.abort();
                if let Err(error) = handle.await
                    && error.is_panic()
                {
                    tracing::debug!(
                        ?error,
                        ?socket_path,
                        "aborted sandbox secret lease daemon task panicked"
                    );
                }
            }
        }
    }
}

/// Inputs `build` needs out of `build_local_runtime`'s local scope. A
/// struct (not four positional params) so Task A6 can add
/// `owner_user_id` and Task A5 can source `activity` from
/// `sandbox_boot`'s returned binding without another signature churn at
/// every call site.
pub(crate) struct SandboxProfileBindingInputs<'a> {
    pub(crate) is_sandboxed_profile: bool,
    pub(crate) local_runtime_identity: Option<&'a RebornLocalRuntimeIdentity>,
    pub(crate) resource_governor: Arc<dyn ResourceGovernor>,
    /// The activity registry the transport (`sandbox_boot::tenant_sandbox_process_binding`)
    /// already constructed and injected into the exec transport — the reaper
    /// must observe the SAME instance, never a second independently
    /// constructed registry, or its idle/activity reads would never match
    /// what the transport records.
    pub(crate) activity: Arc<SandboxActivityRegistry>,
    /// Task A6: the sandbox concurrency ceiling is scoped per-user (not
    /// per-tenant), so one user cannot starve every other user in the
    /// tenant.
    pub(crate) owner_user_id: UserId,
    /// An egress-allowlist proxy `tenant_sandbox_process_binding` already
    /// spawned (and pointed the sandbox container's `default_broker_port`
    /// at) before `build` ever runs — see `sandbox_boot::TenantSandboxBinding::egress_proxy`.
    /// When `Some`, `build` takes ownership of this SAME instance rather
    /// than spawning a second, orphaned proxy; when `None` (e.g. a direct
    /// test construction of `SandboxProfileBindingInputs`, or a future
    /// caller that never pre-spawned one), `build` spawns its own.
    pub(crate) egress_proxy: Option<SandboxEgressProxyRuntimeHandle>,
    /// The `Arc<dyn SecretStore>` instance `factory.rs` already builds and
    /// exposes as `RebornServices.secret_store` — Task 5 threads the SAME
    /// instance into the per-user secret-lease daemon rather than
    /// constructing a second authority.
    pub(crate) secret_store: Arc<dyn SecretStore>,
    /// Root directory the per-user secret-lease socket is bound under:
    /// `sandbox_workspaces_root.join(".ironclaw-broker").join("users").join(<digest>)/broker.sock`.
    pub(crate) sandbox_workspaces_root: PathBuf,
}

pub(crate) struct SandboxRuntimeBindings {
    pub(crate) reaper: Option<crate::sandbox_reaper_task::SandboxReaperRuntimeHandle>,
    pub(crate) egress_proxy: Option<SandboxEgressProxyRuntimeHandle>,
    pub(crate) secret_lease: Option<SandboxSecretLeaseDaemonHandle>,
}

impl SandboxRuntimeBindings {
    /// The non-sandboxed-profile / production-build-context case: no
    /// background tasks.
    pub(crate) fn none() -> Self {
        Self {
            reaper: None,
            egress_proxy: None,
            secret_lease: None,
        }
    }

    /// Builds the sandboxed profile's runtime bindings: applies the
    /// per-user concurrency ceiling and spawns the orphan-container reaper.
    /// Moved verbatim out of `build_local_runtime`'s inline
    /// `if is_sandboxed_profile` block (D3-2/D4-1) — behavior-preserving,
    /// this task only relocates the wiring behind one seam. Non-sandboxed
    /// profiles get `Self::none()` immediately, without touching the
    /// governor or spawning anything.
    pub(crate) async fn build(
        inputs: SandboxProfileBindingInputs<'_>,
    ) -> Result<Self, RebornBuildError> {
        if !inputs.is_sandboxed_profile {
            return Ok(Self::none());
        }

        let sandbox_tenant_id =
            crate::sandbox_quota::resolve_local_runtime_tenant_id(inputs.local_runtime_identity)?;
        // Cloned before `apply_sandbox_user_ceiling` consumes both by value —
        // the secret-lease daemon spawn below needs the same tenant/user
        // pair to key its per-user socket and scope.
        let secret_lease_tenant_id = sandbox_tenant_id.clone();
        let secret_lease_user_id = inputs.owner_user_id.clone();
        crate::sandbox_quota::apply_sandbox_user_ceiling(
            &inputs.resource_governor,
            sandbox_tenant_id,
            inputs.owner_user_id,
            crate::sandbox_quota::sandbox_max_concurrent_from_env(),
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("sandbox user concurrency ceiling could not be set: {error}"),
        })?;

        // D4-1: spawn the orphan-container reaper. Guarded internally —
        // returns `None` rather than failing this build if Docker is not
        // reachable at this (independent, best-effort) connect attempt.
        // Shares `inputs.activity` with the exec transport (Task A5) so
        // both observe the same per-user last-activity timestamps.
        let reaper =
            crate::sandbox_reaper_task::maybe_spawn_sandbox_reaper(Arc::clone(&inputs.activity))
                .await;

        // Phase C: an unbindable egress proxy means sandboxed shell egress
        // would have no enforcement, so — unlike the best-effort reaper —
        // spawn failure here fails this build closed rather than degrading
        // to `None`. Reuse the proxy `tenant_sandbox_process_binding`
        // already spawned (and pointed the container at) when the caller
        // supplied one, rather than binding a second, orphaned proxy nobody
        // shuts down.
        let egress_proxy = match inputs.egress_proxy {
            Some(handle) => Some(handle),
            None => Some(crate::sandbox_egress_proxy_task::spawn_sandbox_egress_proxy().await?),
        };

        // Phase C: the per-user secret-lease daemon. Resolves against the
        // SAME `SecretStore` instance `factory.rs` exposes as
        // `RebornServices.secret_store` — see `CompositionSandboxSecretLeaseResolver`'s
        // doc comment for the documented OAuth-managed-secret gap in this
        // resolver. Fails this build closed on an unbindable socket,
        // mirroring the egress proxy rather than the best-effort reaper.
        let secret_resolver: Arc<dyn ironclaw_host_runtime::SandboxSecretLeaseResolver> = Arc::new(
            crate::sandbox_secret_lease_task::CompositionSandboxSecretLeaseResolver::new(
                Arc::clone(&inputs.secret_store),
            ),
        );
        let secret_lease_scope = ResourceScope {
            tenant_id: secret_lease_tenant_id.clone(),
            user_id: secret_lease_user_id.clone(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let secret_lease_user_key =
            RebornSandboxUserKey::from_tenant_user(&secret_lease_tenant_id, &secret_lease_user_id);
        let sockets_root = inputs.sandbox_workspaces_root.join(".ironclaw-broker");
        let secret_lease = Some(
            crate::sandbox_secret_lease_task::spawn_sandbox_secret_lease_socket(
                secret_resolver,
                secret_lease_scope,
                secret_lease_user_key,
                &sockets_root,
            )
            .await?,
        );

        Ok(Self {
            reaper,
            egress_proxy,
            secret_lease,
        })
    }

    /// The one shutdown call site for every sandbox background task.
    /// `RebornRuntime::shutdown` calls this unconditionally (the struct
    /// is always present on `RebornServices`, never `Option` at that
    /// level — `none()` just means every field inside is empty, so this
    /// is a cheap no-op for non-sandboxed profiles).
    pub(crate) async fn shutdown_all(self, timeout: Duration) {
        if let Some(reaper) = self.reaper {
            reaper.shutdown(timeout).await;
        }
        if let Some(egress_proxy) = self.egress_proxy {
            egress_proxy.shutdown(timeout).await;
        }
        if let Some(secret_lease) = self.secret_lease {
            secret_lease.shutdown(timeout).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{InvocationId, ResourceEstimate, ResourceScope, UserId};
    use ironclaw_resources::InMemoryResourceGovernor;

    fn governor() -> Arc<dyn ironclaw_resources::ResourceGovernor> {
        Arc::new(InMemoryResourceGovernor::new())
    }

    /// A tempdir rooted at `/tmp` rather than `std::env::temp_dir()` — see
    /// `RebornSandboxUserKey::socket_path`'s doc comment: macOS's `TMPDIR`
    /// is a deep, per-process, randomized path that alone can exhaust a
    /// Unix socket's ~104-byte `sun_path` budget before this module adds
    /// its own `.ironclaw-broker/users/<digest>/broker.sock` suffix.
    fn short_tempdir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("ic-sandbox-")
            .tempdir_in("/tmp")
            .expect("short tempdir under /tmp")
    }

    #[tokio::test]
    async fn non_sandboxed_profile_yields_inert_bindings_with_no_reaper() {
        let sockets_root = short_tempdir();
        let bindings = SandboxRuntimeBindings::build(SandboxProfileBindingInputs {
            is_sandboxed_profile: false,
            local_runtime_identity: None,
            resource_governor: governor(),
            activity: Arc::new(SandboxActivityRegistry::new()),
            owner_user_id: UserId::new("probe-user").unwrap(),
            egress_proxy: None,
            secret_store: Arc::new(ironclaw_secrets::FilesystemSecretStore::ephemeral()),
            sandbox_workspaces_root: sockets_root.path().to_path_buf(),
        })
        .await
        .expect("non-sandboxed profile never fails to build inert bindings");

        assert!(bindings.reaper.is_none());
        assert!(bindings.egress_proxy.is_none());
        assert!(bindings.secret_lease.is_none());
        bindings.shutdown_all(Duration::from_secs(1)).await;
    }

    #[tokio::test]
    async fn sandboxed_profile_applies_the_user_ceiling() {
        let governor = governor();
        let owner_user_id = UserId::new("probe-user").unwrap();
        let sockets_root = short_tempdir();
        let bindings = SandboxRuntimeBindings::build(SandboxProfileBindingInputs {
            is_sandboxed_profile: true,
            local_runtime_identity: None,
            resource_governor: Arc::clone(&governor),
            activity: Arc::new(SandboxActivityRegistry::new()),
            owner_user_id: owner_user_id.clone(),
            egress_proxy: None,
            secret_store: Arc::new(ironclaw_secrets::FilesystemSecretStore::ephemeral()),
            sandbox_workspaces_root: sockets_root.path().to_path_buf(),
        })
        .await
        .expect("sandboxed profile build succeeds even with no reachable Docker daemon");

        // The ceiling is live regardless of whether the reaper spawn found
        // a Docker daemon (best-effort, mirrors
        // `sandbox_reaper_task::tests::no_docker_daemon_yields_no_handle`).
        let tenant_id = crate::sandbox_quota::resolve_local_runtime_tenant_id(None).unwrap();
        let scope = ResourceScope {
            tenant_id,
            user_id: owner_user_id,
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let first = governor
            .reserve(scope, ResourceEstimate::default().set_concurrency_slots(1))
            .expect("first reservation is within the default ceiling");
        drop(first);

        bindings.shutdown_all(Duration::from_secs(1)).await;
    }

    #[test]
    fn none_constructor_produces_no_handles() {
        let bindings = SandboxRuntimeBindings::none();
        assert!(bindings.reaper.is_none());
        assert!(bindings.egress_proxy.is_none());
        assert!(bindings.secret_lease.is_none());
    }
}
