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

use ironclaw_host_runtime::{RebornSandboxConfig, RebornScopedSandboxCommandTransport};

use crate::RebornBuildError;
use crate::input::RebornRuntimeProcessBinding;

/// Full proxy URL (e.g. `http://allowlist-proxy.internal:3128`) the
/// sandboxed shell container's `http_proxy`/`https_proxy` env should point
/// at. Takes priority over [`SANDBOX_HTTP_PROXY_PORT_ENV`] when both are set.
///
/// Soft-enforcement model (mirrors legacy IronClaw's sandbox): the container
/// keeps normal bridge networking and is steered through this proxy, which
/// is expected to enforce the egress allowlist
/// (`ironclaw_host_runtime::sandbox_allowed_domains`) — see the follow-up
/// note below.
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
/// registries (`ironclaw_host_runtime::sandbox_allowed_domains`). Without
/// either env var the sandbox falls back to the prior `--network none`
/// posture — no egress at all, but still a safe default rather than a
/// build failure — until a proxy address is configured.
///
/// TODO(follow-up, not built here): this only points the container at a
/// proxy address; it does not stand one up. The actual host-side allowlist
/// **proxy server** — a forward proxy that enforces
/// `ironclaw_host_runtime::sandbox_allowed_domains` and is reachable from
/// the sandbox container at the configured address — still needs to be
/// built and deployed. Until it lands, setting either env var here routes
/// the container's traffic at *something*, but that something must already
/// exist and enforce the allowlist, or the container effectively gets open
/// egress.
pub async fn tenant_sandbox_process_binding(
    sandbox_workspaces_root: PathBuf,
) -> Result<RebornRuntimeProcessBinding, RebornBuildError> {
    let config = with_sandbox_network_broker(RebornSandboxConfig::new(sandbox_workspaces_root))?;
    let transport = RebornScopedSandboxCommandTransport::connect(config)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!(
                "tenant-sandbox process backend requires a reachable Docker daemon: {error}"
            ),
        })?;
    let process_port = Arc::new(transport.into_process_port());
    Ok(RebornRuntimeProcessBinding::tenant_sandbox(process_port))
}

/// Applies the configured proxy broker (if any) to `config`. Missing or
/// invalid configuration is handled gracefully: an absent env var leaves the
/// sandbox at its safe `--network none` default rather than failing the
/// build; an invalid `SANDBOX_HTTP_PROXY_URL_ENV` value fails closed with a
/// descriptive error rather than silently falling back to no proxy (which
/// would look configured but silently grant no egress instead of the
/// intended allowlisted egress).
fn with_sandbox_network_broker(
    config: RebornSandboxConfig,
) -> Result<RebornSandboxConfig, RebornBuildError> {
    with_sandbox_network_broker_from_values(
        config,
        std::env::var(SANDBOX_HTTP_PROXY_URL_ENV).ok(),
        std::env::var(SANDBOX_HTTP_PROXY_PORT_ENV).ok(),
    )
}

/// Inner resolver taking the raw proxy env values as parameters so tests can
/// drive every branch without mutating process-global env — this crate is
/// `#![forbid(unsafe_code)]`, which bans `std::env::set_var`. Mirrors the
/// `hosted_volume_secret_master_key_from_raw` pattern in `deployment.rs`.
fn with_sandbox_network_broker_from_values(
    config: RebornSandboxConfig,
    proxy_url: Option<String>,
    proxy_port: Option<String>,
) -> Result<RebornSandboxConfig, RebornBuildError> {
    if let Some(proxy_url) = proxy_url {
        return config
            .with_network_broker_proxy_url(proxy_url)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!(
                    "{SANDBOX_HTTP_PROXY_URL_ENV} is not a usable proxy URL: {error}"
                ),
            });
    }
    if let Some(raw_port) = proxy_port {
        let port = raw_port
            .trim()
            .parse::<u16>()
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!(
                    "{SANDBOX_HTTP_PROXY_PORT_ENV} must be a valid port number: {error}"
                ),
            })?;
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
        )
        .expect("valid proxy port wires successfully");

        assert!(!format!("{config:?}").contains("network_broker: None"));
    }
}
