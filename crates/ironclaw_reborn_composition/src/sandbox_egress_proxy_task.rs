//! Composition-owned spawn of `ironclaw_host_runtime::EgressAllowlistProxy`
//! (Phase C, Task 2).
//!
//! Mirrors `sandbox_reaper_task.rs`'s split exactly: the proxy core
//! (`ironclaw_host_runtime::sandbox_process::egress_proxy`) is deliberately
//! unopinionated about scheduling — this module connects it to a real bind
//! address, spawns its accept loop as a background task, and owns its
//! cancellation via [`SandboxEgressProxyRuntimeHandle`].
//!
//! **Bind-address decision (2026-07-22 review finding #8, verified against
//! `broker.rs`):** production binds `0.0.0.0:0`, never `127.0.0.1:0`. The
//! sandboxed container is steered at the proxy via the Docker bridge
//! gateway address (`172.17.0.1:<port>` — see `docker_host_gateway()` at
//! `crates/ironclaw_host_runtime/src/sandbox_process/broker.rs:339-345`), and
//! a listener bound to loopback never accepts bridge-destined traffic
//! (standard socket semantics) — a loopback bind is broken-by-construction
//! for this use on Linux. The allowlist policy itself is the access
//! control; the wider bind exposes only the policy-enforcing proxy.
//!
//! **Fail-closed, unlike the reaper:** an unbindable egress proxy means the
//! sandboxed profile's shell egress would have no enforcement at all, so
//! [`spawn_sandbox_egress_proxy`] returns `Err` on bind failure rather than
//! `None` — `SandboxRuntimeBindings::build` propagates that error and fails
//! the whole sandboxed-profile build closed.

use ironclaw_host_runtime::EgressAllowlistProxy;
use tokio::sync::watch;

use crate::RebornBuildError;
use crate::sandbox_composition::SandboxEgressProxyRuntimeHandle;

/// Production bind address: all interfaces, OS-chosen ephemeral port. See
/// the module doc for why this must not be `127.0.0.1`.
const EGRESS_PROXY_BIND_ADDR: &str = "0.0.0.0:0";

/// Binds the egress allowlist proxy to [`EGRESS_PROXY_BIND_ADDR`] and spawns
/// its accept loop as an owned background task. No Docker dependency — this
/// always attempts the bind regardless of whether a Docker daemon is
/// reachable.
pub(crate) async fn spawn_sandbox_egress_proxy()
-> Result<SandboxEgressProxyRuntimeHandle, RebornBuildError> {
    let policy = ironclaw_host_runtime::sandbox_network_policy();
    let bound = EgressAllowlistProxy::new(policy)
        .bind(EGRESS_PROXY_BIND_ADDR)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("sandbox egress allowlist proxy failed to bind: {error}"),
        })?;
    let local_addr = bound.local_addr();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let handle = tokio::spawn(async move {
        bound.serve(shutdown_rx).await;
    });

    Ok(SandboxEgressProxyRuntimeHandle::new(
        shutdown_tx,
        handle,
        local_addr,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::net::TcpStream;

    const TEST_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

    #[tokio::test]
    async fn spawn_returns_a_bound_reachable_handle() {
        let handle = spawn_sandbox_egress_proxy()
            .await
            .expect("binding an ephemeral port always succeeds");
        let local_addr = handle.local_addr;

        // 0.0.0.0:0 binds all interfaces; dial back via loopback (which is
        // one of the interfaces a 0.0.0.0 bind accepts on) to prove the
        // accept loop is live, not just that a listener exists.
        let dial_addr = std::net::SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            local_addr.port(),
        );
        let connected = TcpStream::connect(dial_addr).await;
        assert!(
            connected.is_ok(),
            "expected to connect to the spawned proxy at {dial_addr}: {connected:?}"
        );

        handle.shutdown(TEST_SHUTDOWN_TIMEOUT).await;
    }

    /// Mirrors `sandbox_reaper_task::tests::shutdown_stops_a_running_task_before_the_timeout`,
    /// driving `spawn_sandbox_egress_proxy`'s real future (no Docker needed
    /// for a proxy bind, unlike the reaper).
    #[tokio::test]
    async fn shutdown_stops_a_running_task_before_the_timeout() {
        let handle = spawn_sandbox_egress_proxy()
            .await
            .expect("binding an ephemeral port always succeeds");

        handle.shutdown(TEST_SHUTDOWN_TIMEOUT).await;
        // Reaching here without hanging proves the shutdown signal reached
        // the task and the join completed inside the timeout.
    }
}
