//! Boot-time construction of the `TenantSandbox` process-port binding for
//! composition profiles that request it (today: only
//! `hosted-single-tenant-volume-sandboxed`).
//!
//! This is the one place composition reaches into
//! `ironclaw_host_runtime`'s sandbox transport to produce a
//! [`RebornRuntimeProcessBinding`]. Callers above composition — notably the
//! `ironclaw` CLI binary crate — enter Reborn only through this crate's
//! public surface and must never depend on `ironclaw_host_runtime` directly
//! (enforced by
//! `ironclaw_architecture::reborn_cli_binary_crate_stays_separate_from_v1_root`),
//! so the Docker connect + transport construction lives here rather than at
//! the CLI boot-input assembly call site.

use std::path::PathBuf;
use std::sync::Arc;

use ironclaw_host_runtime::{
    RebornSandboxConfig, RebornScopedSandboxCommandTransport, SandboxActivityRegistry,
};

use crate::RebornBuildError;
use crate::input::RebornRuntimeProcessBinding;

/// Full proxy URL (e.g. `http://allowlist-proxy.internal:3128`) the
/// sandboxed shell container's `http_proxy`/`https_proxy` env should point
/// at. Takes priority over [`SANDBOX_HTTP_PROXY_PORT_ENV`] when both are set.
///
/// Hard (topological) enforcement model: the sandboxed container is placed
/// on a pinned, Docker `internal: true` network with no default route off
/// the host (`ironclaw_host_runtime::sandbox_process::exec_transport`'s
/// `SANDBOX_EGRESS_NETWORK_NAME`), so this proxy is the container's only
/// path to the outside world — not a steering convention layered on normal
/// bridge networking. It enforces the egress allowlist
/// (`ironclaw_host_runtime::sandbox_process::egress_proxy`).
const SANDBOX_HTTP_PROXY_URL_ENV: &str = "IRONCLAW_SANDBOX_HTTP_PROXY";

/// Port of an allowlist proxy reachable via the Docker host-gateway address
/// (`172.17.0.1` on Linux, `host.docker.internal` elsewhere — see
/// `RebornSandboxConfig::with_network_broker_port`). Used only when
/// [`SANDBOX_HTTP_PROXY_URL_ENV`] is unset; lets an operator run the proxy on
/// the host without hardcoding its address.
const SANDBOX_HTTP_PROXY_PORT_ENV: &str = "IRONCLAW_SANDBOX_HTTP_PROXY_PORT";

/// Connect to the Docker daemon and build a `TenantSandbox` process-port
/// binding rooted at `sandbox_workspaces_root`. Fails closed: any Docker
/// connect failure returns `Err`, never a silent
/// `RebornRuntimeProcessBinding::none()` fallback (which would mean running
/// sandbox-profile shell commands unsandboxed on the host) — see
/// `docs/safety-and-sandbox.md`.
///
/// Network egress: if [`SANDBOX_HTTP_PROXY_URL_ENV`] or
/// [`SANDBOX_HTTP_PROXY_PORT_ENV`] names a reachable proxy, the container
/// gets normal (bridge) networking plus `http_proxy`/`https_proxy` env
/// pointing at it, so pip/npm/git/curl workflows can reach the allowlisted
/// registries. Without either env var, `default_broker_port` is used when
/// present; when it is also `None`, this function spawns a fresh
/// [`ironclaw_host_runtime::EgressAllowlistProxy`] itself (fail-closed: a
/// bind failure here fails this call, never a silent `--network none`
/// downgrade masquerading as "configured") and uses its freshly bound port,
/// so unconfigured deployments still get a live, allowlist-enforcing
/// default rather than the prior `--network none` posture. A caller that
/// already spawned (and separately owns the lifecycle of) a proxy — e.g.
/// composition's boot path, which threads the resulting handle onward via
/// [`TenantSandboxBinding::egress_proxy`] for `SandboxRuntimeBindings` to
/// shut down — passes that proxy's port as `default_broker_port` instead of
/// asking this function to spawn a second, orphaned one.
pub async fn tenant_sandbox_process_binding(
    sandbox_workspaces_root: PathBuf,
    default_broker_port: Option<u16>,
) -> Result<TenantSandboxBinding, RebornBuildError> {
    let (default_broker_port, egress_proxy) = match default_broker_port {
        Some(port) => (Some(port), None),
        None => {
            let handle = crate::sandbox_egress_proxy_task::spawn_sandbox_egress_proxy().await?;
            let port = handle.local_addr.port();
            (Some(port), Some(handle))
        }
    };
    let config = with_sandbox_network_broker(
        RebornSandboxConfig::new(sandbox_workspaces_root),
        default_broker_port,
    )?;
    let activity = Arc::new(SandboxActivityRegistry::new());
    let transport = RebornScopedSandboxCommandTransport::connect(config)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!(
                "tenant-sandbox process backend requires a reachable Docker daemon: {error}"
            ),
        })?
        .with_activity_registry(Arc::clone(&activity));
    let process_port = Arc::new(transport.into_process_port());
    Ok(TenantSandboxBinding {
        binding: RebornRuntimeProcessBinding::tenant_sandbox(process_port),
        activity,
        egress_proxy,
    })
}

/// Return value of [`tenant_sandbox_process_binding`]: the process-port
/// binding plus the SAME [`SandboxActivityRegistry`] instance the exec
/// transport now writes activity into, so a caller that also spawns
/// `SandboxReaper` (via `sandbox_composition`/`factory.rs`) reads the exact
/// timestamps the transport is recording — never a second, independently
/// constructed registry.
pub struct TenantSandboxBinding {
    pub binding: RebornRuntimeProcessBinding,
    pub activity: Arc<SandboxActivityRegistry>,
    /// `Some` when this call spawned its own egress-allowlist proxy (the
    /// production case: no `default_broker_port` was supplied). The caller
    /// threads this onward (`RebornBuildInput::with_sandbox_egress_proxy_handle`)
    /// so `SandboxRuntimeBindings::build` takes ownership of the SAME
    /// instance rather than spawning a second one — one bound proxy per
    /// sandboxed-profile boot, one owner for its shutdown.
    pub egress_proxy: Option<crate::sandbox_composition::SandboxEgressProxyRuntimeHandle>,
}

/// Applies the configured proxy broker (if any) to `config`. Missing or
/// invalid configuration is handled gracefully: an absent env var falls
/// back to `default_port` (see [`tenant_sandbox_process_binding`]) rather
/// than failing the build; an invalid `SANDBOX_HTTP_PROXY_URL_ENV` value
/// fails closed with a descriptive error rather than silently falling back
/// to no proxy (which would look configured but silently grant no egress
/// instead of the intended allowlisted egress).
fn with_sandbox_network_broker(
    config: RebornSandboxConfig,
    default_port: Option<u16>,
) -> Result<RebornSandboxConfig, RebornBuildError> {
    with_sandbox_network_broker_from_values(
        config,
        std::env::var(SANDBOX_HTTP_PROXY_URL_ENV).ok(),
        std::env::var(SANDBOX_HTTP_PROXY_PORT_ENV).ok(),
        default_port,
    )
}

/// Inner resolver taking the raw proxy env values as parameters so tests can
/// drive every branch without mutating process-global env — this crate is
/// `#![forbid(unsafe_code)]`, which bans `std::env::set_var`. Mirrors the
/// `hosted_volume_secret_master_key_from_raw` pattern in `deployment.rs`.
///
/// Precedence: `proxy_url` env wins over `proxy_port` env, which wins over
/// `default_port` — an operator-pointed external proxy is never overridden
/// by the composition-supplied default.
fn with_sandbox_network_broker_from_values(
    config: RebornSandboxConfig,
    proxy_url: Option<String>,
    proxy_port: Option<String>,
    default_port: Option<u16>,
) -> Result<RebornSandboxConfig, RebornBuildError> {
    if let Some(proxy_url) = proxy_url {
        return config
            .with_network_broker_proxy_url(proxy_url)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("{SANDBOX_HTTP_PROXY_URL_ENV} is not a usable proxy URL: {error}"),
            });
    }
    if let Some(raw_port) = proxy_port {
        let port =
            raw_port
                .trim()
                .parse::<u16>()
                .map_err(|error| RebornBuildError::InvalidConfig {
                    reason: format!(
                        "{SANDBOX_HTTP_PROXY_PORT_ENV} must be a valid port number: {error}"
                    ),
                })?;
        return Ok(config.with_network_broker_port(port));
    }
    if let Some(port) = default_port {
        return Ok(config.with_network_broker_port(port));
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    // These drive `with_sandbox_network_broker_from_values` directly with
    // explicit env values so no process-global env mutation is needed — this
    // crate is `#![forbid(unsafe_code)]`, which bans `std::env::set_var`.

    #[test]
    fn missing_proxy_env_leaves_sandbox_at_safe_default() {
        let config = with_sandbox_network_broker_from_values(
            RebornSandboxConfig::new("/tmp/reborn-sandbox"),
            None,
            None,
            None,
        )
        .expect("no proxy env configured is not an error");

        // No broker configured: the config's Debug output still shows the
        // safe `network_broker: None` default.
        assert!(format!("{config:?}").contains("network_broker: None"));
    }

    #[test]
    fn proxy_url_env_wires_a_network_broker() {
        let config = with_sandbox_network_broker_from_values(
            RebornSandboxConfig::new("/tmp/reborn-sandbox"),
            Some("http://proxy.internal:3128".to_string()),
            None,
            None,
        )
        .expect("valid proxy URL wires successfully");

        assert!(!format!("{config:?}").contains("network_broker: None"));
    }

    #[test]
    fn invalid_proxy_url_env_fails_closed() {
        let result = with_sandbox_network_broker_from_values(
            RebornSandboxConfig::new("/tmp/reborn-sandbox"),
            Some("not a url".to_string()),
            None,
            None,
        );

        assert!(
            result.is_err(),
            "an unusable proxy URL must fail closed, not silently drop the broker"
        );
    }

    #[test]
    fn proxy_port_env_wires_a_network_broker_via_host_gateway() {
        let config = with_sandbox_network_broker_from_values(
            RebornSandboxConfig::new("/tmp/reborn-sandbox"),
            None,
            Some("8181".to_string()),
            None,
        )
        .expect("valid proxy port wires successfully");

        assert!(!format!("{config:?}").contains("network_broker: None"));
    }

    #[test]
    fn default_port_env_absent_falls_back_to_supplied_default_port() {
        let config = with_sandbox_network_broker_from_values(
            RebornSandboxConfig::new("/tmp/reborn-sandbox"),
            None,
            None,
            Some(8181),
        )
        .expect("a supplied default port wires successfully with no env set");

        assert!(!format!("{config:?}").contains("network_broker: None"));
    }

    #[test]
    fn explicit_proxy_url_env_still_wins_over_default_port() {
        let with_default_only = with_sandbox_network_broker_from_values(
            RebornSandboxConfig::new("/tmp/reborn-sandbox"),
            None,
            None,
            Some(8181),
        )
        .expect("default port wires successfully");

        let with_env_and_default = with_sandbox_network_broker_from_values(
            RebornSandboxConfig::new("/tmp/reborn-sandbox"),
            Some("http://proxy.internal:3128".to_string()),
            None,
            Some(8181),
        )
        .expect("env proxy URL wires successfully even with a default port supplied");

        // The env-sourced broker (a proxy URL) must differ from the
        // default-port-only broker (a host-gateway port) — proves env
        // precedence took effect rather than the default silently winning.
        assert_ne!(
            format!("{with_default_only:?}"),
            format!("{with_env_and_default:?}"),
            "explicit proxy URL env must produce a different broker than the default port alone"
        );
    }
}
