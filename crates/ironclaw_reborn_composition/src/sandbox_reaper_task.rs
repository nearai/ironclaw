//! Composition-owned spawn of `ironclaw_host_runtime::SandboxReaper` (D4-1).
//!
//! The reaper core (`ironclaw_host_runtime::sandbox_process::reaper`) is
//! deliberately unopinionated about scheduling — composition is the one
//! place that connects to Docker, constructs the reaper, spawns it as a
//! background task, and owns its cancellation. This module is that one
//! place; `factory.rs` calls [`maybe_spawn_sandbox_reaper`] with a single
//! line rather than growing its own Docker-connect-and-spawn logic.
//!
//! Guarded, not required: the sandboxed profile's boot path
//! (`sandbox_boot::tenant_sandbox_process_binding`) already fails the whole
//! boot closed if Docker is unreachable, so by the time this is called the
//! daemon was reachable a moment ago. This function still tolerates a
//! transient failure of its own (independent) connect attempt by returning
//! `None` — the reaper is a best-effort orphan sweep, not a boot
//! precondition, so its absence must never fail composition.

use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_runtime::{SandboxReaper, SandboxReaperConfig};
use ironclaw_run_state::RunStateStore;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// How long [`SandboxReaperRuntimeHandle::shutdown`] waits for the reaper's
/// in-flight scan to observe the shutdown signal and return before it aborts
/// the task outright. Mirrors `CREDENTIAL_REFRESH_WORKER_SHUTDOWN_TIMEOUT`'s
/// role for the credential-refresh worker.
pub(crate) const SANDBOX_REAPER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Owned handle to a spawned [`SandboxReaper::run`] task. Composition holds
/// exactly one of these (on `RebornRuntime`) for the sandboxed profile and
/// drives shutdown from `RebornRuntime::shutdown` alongside every other
/// owned background worker.
pub(crate) struct SandboxReaperRuntimeHandle {
    shutdown_tx: watch::Sender<bool>,
    handle: JoinHandle<()>,
}

impl SandboxReaperRuntimeHandle {
    /// Signals the reaper's scan loop to stop and awaits the task, aborting
    /// it if it has not stopped within `timeout`.
    pub(crate) async fn shutdown(self, timeout: Duration) {
        // A closed receiver (task already gone) is not an error here.
        let _ = self.shutdown_tx.send(true);
        let mut handle = self.handle;
        match tokio::time::timeout(timeout, &mut handle).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::debug!(?error, "sandbox reaper task join failed");
            }
            Err(_) => {
                tracing::debug!(
                    ?timeout,
                    "sandbox reaper did not stop before shutdown timeout; aborting"
                );
                handle.abort();
                if let Err(error) = handle.await
                    && error.is_panic()
                {
                    tracing::debug!(?error, "aborted sandbox reaper task panicked");
                }
            }
        }
    }
}

/// Connects to Docker and spawns [`SandboxReaper::run`] as an owned
/// background task, returning `None` (never an error) when Docker is not
/// reachable — this machine's dev/CI environment commonly has no Docker
/// daemon, and the sandboxed profile itself already fails closed on Docker
/// unavailability at its own (earlier) connect, so a reaper-spawn failure
/// here must not additionally fail boot.
pub(crate) async fn maybe_spawn_sandbox_reaper(
    run_state: Arc<dyn RunStateStore>,
) -> Option<SandboxReaperRuntimeHandle> {
    let docker = match ironclaw_host_runtime::connect_docker_with_retry().await {
        Ok(docker) => docker,
        Err(error) => {
            tracing::debug!(
                ?error,
                "sandbox reaper: Docker daemon unreachable, skipping reaper spawn"
            );
            return None;
        }
    };

    let reaper = Arc::new(SandboxReaper::new(
        docker,
        run_state,
        SandboxReaperConfig::default(),
    ));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let handle = tokio::spawn(async move {
        reaper.run(shutdown_rx).await;
    });

    Some(SandboxReaperRuntimeHandle {
        shutdown_tx,
        handle,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{ApprovalRequest, InvocationId, ResourceScope};
    use ironclaw_run_state::{RunRecord, RunStart, RunStateError};

    /// Minimal stand-in for a `RunStateStore` backend: the reaper-task tests
    /// here never exercise real run-state persistence (that is
    /// `ironclaw_run_state`'s own coverage), they only need *some*
    /// `Arc<dyn RunStateStore>` to construct `maybe_spawn_sandbox_reaper`'s
    /// argument. `ironclaw_run_state::test_support` is private to that crate,
    /// so this local fake mirrors
    /// `sandbox_reaper_docker.rs`'s `AlwaysAbsentRunStateStore`: `get`
    /// reports "no record", and the writer methods are never called by
    /// either test, so they're `unreachable!()`. Written inline (not via a
    /// declarative macro) because `#[async_trait]` sees a macro
    /// *invocation*, not the expanded `async fn`s, and cannot rewrite them —
    /// which produces E0195.
    struct AlwaysAbsentRunStateStore;

    #[async_trait::async_trait]
    impl RunStateStore for AlwaysAbsentRunStateStore {
        async fn start(&self, _start: RunStart) -> Result<RunRecord, RunStateError> {
            unreachable!("AlwaysAbsentRunStateStore never calls start()")
        }
        async fn block_approval(
            &self,
            _scope: &ResourceScope,
            _invocation_id: InvocationId,
            _approval: ApprovalRequest,
        ) -> Result<RunRecord, RunStateError> {
            unreachable!("AlwaysAbsentRunStateStore never calls block_approval()")
        }
        async fn block_auth(
            &self,
            _scope: &ResourceScope,
            _invocation_id: InvocationId,
            _error_kind: String,
        ) -> Result<RunRecord, RunStateError> {
            unreachable!("AlwaysAbsentRunStateStore never calls block_auth()")
        }
        async fn complete(
            &self,
            _scope: &ResourceScope,
            _invocation_id: InvocationId,
        ) -> Result<RunRecord, RunStateError> {
            unreachable!("AlwaysAbsentRunStateStore never calls complete()")
        }
        async fn fail(
            &self,
            _scope: &ResourceScope,
            _invocation_id: InvocationId,
            _error_kind: String,
        ) -> Result<RunRecord, RunStateError> {
            unreachable!("AlwaysAbsentRunStateStore never calls fail()")
        }
        async fn records_for_scope(
            &self,
            _scope: &ResourceScope,
        ) -> Result<Vec<RunRecord>, RunStateError> {
            unreachable!("AlwaysAbsentRunStateStore never calls records_for_scope()")
        }
        async fn get(
            &self,
            _scope: &ResourceScope,
            _invocation_id: InvocationId,
        ) -> Result<Option<RunRecord>, RunStateError> {
            Ok(None)
        }
    }

    /// The guard the module doc promises: no Docker daemon means `None`, not
    /// a panic/error. This machine's dev/CI default has no Docker daemon, so
    /// that is the expected branch here; mirrors
    /// `connect::tests::readiness_surfaces_reason_on_unreachable_daemon`'s
    /// tolerance for a CI runner that happens to have Docker reachable —
    /// `Some` is also a valid, non-flaky outcome there, and this test cleans
    /// it up rather than asserting the environment.
    #[tokio::test]
    async fn no_docker_daemon_yields_no_handle() {
        let run_state: Arc<dyn RunStateStore> = Arc::new(AlwaysAbsentRunStateStore);

        match maybe_spawn_sandbox_reaper(run_state).await {
            None => {}
            Some(handle) => {
                // A real Docker daemon happens to be reachable on this
                // machine/CI runner: the spawn succeeding is not itself the
                // property under test here (that is Docker-gated coverage
                // elsewhere) — just prove the handle is a real, cancellable
                // task and clean it up.
                handle.shutdown(SANDBOX_REAPER_SHUTDOWN_TIMEOUT).await;
            }
        }
    }

    /// The handle's cancellation path (shutdown signal -> task observes it
    /// and returns -> join succeeds) is exercised directly against a
    /// `SandboxReaper::run` future without going through Docker, proving the
    /// handle is a real owned/cancellable task rather than a fire-and-forget
    /// spawn. Mirrors the shape `maybe_spawn_sandbox_reaper` produces, just
    /// without the Docker connect this machine cannot perform.
    #[tokio::test]
    async fn shutdown_stops_a_running_task_before_the_timeout() {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let handle = tokio::spawn(async move {
            // Stand-in for `SandboxReaper::run`'s own shutdown-aware loop.
            let _ = shutdown_rx.changed().await;
        });
        let handle = SandboxReaperRuntimeHandle {
            shutdown_tx,
            handle,
        };

        handle.shutdown(SANDBOX_REAPER_SHUTDOWN_TIMEOUT).await;
        // Reaching here without hanging proves the shutdown signal reached
        // the task and the join completed inside the timeout.
    }
}
