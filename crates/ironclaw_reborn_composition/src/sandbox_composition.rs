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

use ironclaw_host_runtime::SandboxActivityRegistry;
use ironclaw_resources::ResourceGovernor;

use crate::RebornBuildError;
use crate::input::RebornLocalRuntimeIdentity;

/// Reserved for Phase C (secret-broker + egress-allowlist proxy daemons).
/// Declared now so `SandboxRuntimeBindings`'s shape does not change again
/// when Phase C lands. Both handle structs are declared canonically here
/// with their real fields (shutdown_tx/handle + local_addr/socket_path,
/// mirroring `SandboxReaperRuntimeHandle`); Phase A only ever sets them to
/// `None`. Phase C imports these types from `sandbox_composition` and
/// constructs `Some(..)` — it does NOT redeclare same-named structs.
// Constructed by Phase C (egress proxy / secret-lease daemon); reserved here so
// SandboxRuntimeBindings's shape is stable.
#[allow(dead_code)]
pub(crate) struct SandboxEgressProxyRuntimeHandle {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    handle: tokio::task::JoinHandle<()>,
    pub(crate) local_addr: std::net::SocketAddr,
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
    /// tenant concurrency ceiling and spawns the orphan-container reaper.
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
        crate::sandbox_quota::apply_sandbox_tenant_ceiling(
            &inputs.resource_governor,
            sandbox_tenant_id,
            crate::sandbox_quota::sandbox_max_concurrent_from_env(),
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("sandbox tenant concurrency ceiling could not be set: {error}"),
        })?;

        // D4-1: spawn the orphan-container reaper. Guarded internally —
        // returns `None` rather than failing this build if Docker is not
        // reachable at this (independent, best-effort) connect attempt.
        // Shares `inputs.activity` with the exec transport (Task A5) so
        // both observe the same per-user last-activity timestamps.
        let reaper =
            crate::sandbox_reaper_task::maybe_spawn_sandbox_reaper(Arc::clone(&inputs.activity))
                .await;

        Ok(Self {
            activity: inputs.activity,
            reaper,
            egress_proxy: None,
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
        // Phase C: egress_proxy/secret_lease daemon shutdown joins here
        // too, once those variants are ever `Some`.
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
        })
        .await
        .expect("non-sandboxed profile never fails to build inert bindings");

        assert!(bindings.reaper.is_none());
        assert!(bindings.egress_proxy.is_none());
        assert!(bindings.secret_lease.is_none());
        bindings.shutdown_all(Duration::from_secs(1)).await;
    }

    #[tokio::test]
    async fn sandboxed_profile_applies_the_tenant_ceiling() {
        let governor = governor();
        let bindings = SandboxRuntimeBindings::build(SandboxProfileBindingInputs {
            is_sandboxed_profile: true,
            local_runtime_identity: None,
            resource_governor: Arc::clone(&governor),
            activity: Arc::new(SandboxActivityRegistry::new()),
        })
        .await
        .expect("sandboxed profile build succeeds even with no reachable Docker daemon");

        // The ceiling is live regardless of whether the reaper spawn found
        // a Docker daemon (best-effort, mirrors
        // `sandbox_reaper_task::tests::no_docker_daemon_yields_no_handle`).
        let tenant_id = crate::sandbox_quota::resolve_local_runtime_tenant_id(None).unwrap();
        let scope = ResourceScope {
            tenant_id,
            user_id: UserId::new("probe-user").unwrap(),
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
