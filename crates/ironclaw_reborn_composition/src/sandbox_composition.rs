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

use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_api::UserId;
use ironclaw_host_runtime::SandboxActivityRegistry;
use ironclaw_resources::ResourceGovernor;

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

// Constructed by Phase C (egress proxy / secret-lease daemon); reserved here so
// SandboxRuntimeBindings's shape is stable.
#[allow(dead_code)]
pub(crate) struct SandboxSecretLeaseDaemonHandle {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    handle: tokio::task::JoinHandle<()>,
    pub(crate) socket_path: std::path::PathBuf,
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
}

pub(crate) struct SandboxRuntimeBindings {
    pub(crate) activity: Arc<SandboxActivityRegistry>,
    pub(crate) reaper: Option<crate::sandbox_reaper_task::SandboxReaperRuntimeHandle>,
    pub(crate) egress_proxy: Option<SandboxEgressProxyRuntimeHandle>,
    pub(crate) secret_lease: Option<SandboxSecretLeaseDaemonHandle>,
}

impl SandboxRuntimeBindings {
    /// The non-sandboxed-profile / production-build-context case: no
    /// background tasks. The activity registry is still real (not
    /// `Option`) so the field never needs unwrapping at call sites — it
    /// is simply never touched by anything for these profiles.
    pub(crate) fn none() -> Self {
        Self {
            activity: Arc::new(SandboxActivityRegistry::new()),
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

        Ok(Self {
            activity: inputs.activity,
            reaper,
            egress_proxy,
            secret_lease: None,
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
        // Phase C: secret_lease daemon shutdown joins here too, once that
        // variant is ever `Some`.
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

    #[tokio::test]
    async fn non_sandboxed_profile_yields_inert_bindings_with_no_reaper() {
        let bindings = SandboxRuntimeBindings::build(SandboxProfileBindingInputs {
            is_sandboxed_profile: false,
            local_runtime_identity: None,
            resource_governor: governor(),
            activity: Arc::new(SandboxActivityRegistry::new()),
            owner_user_id: UserId::new("probe-user").unwrap(),
            egress_proxy: None,
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
        let bindings = SandboxRuntimeBindings::build(SandboxProfileBindingInputs {
            is_sandboxed_profile: true,
            local_runtime_identity: None,
            resource_governor: Arc::clone(&governor),
            activity: Arc::new(SandboxActivityRegistry::new()),
            owner_user_id: owner_user_id.clone(),
            egress_proxy: None,
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
