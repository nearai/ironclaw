//! Launches `ironclaw-reborn serve` when it isn't already reachable, and
//! polls the launched (or already-running) process until it answers
//! `/api/webchat/v2/session`. [`ensure_serve`] is the entry point `lib.rs`'s
//! `run_tui` calls before starting the terminal event loop; the readiness
//! poll itself is factored into `poll_until_ready` (`pub(crate)`,
//! parameterized by `Duration`) so tests can drive it with a tiny budget
//! and interval instead of waiting out the real 15s/300ms constants.

use std::path::PathBuf;
use std::time::Duration;

use crate::client::{ApiClient, ClientError};

/// Plain-data description of how to (re-)launch `ironclaw-reborn serve`,
/// handed in by the CLI's `tui` subcommand entry point. This crate never
/// resolves its own exe path or constructs Reborn env vars — see
/// `ironclaw_reborn_cli::serve_invocation` and the "TUI never calls CLI
/// code" note in the design doc's Part A.
#[derive(Debug, Clone)]
pub struct ProcessInvocation {
    pub exe: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("serve is unreachable and no spawn was configured")]
    NoServeAvailable,
    #[error("failed to spawn serve child process: {0}")]
    SpawnFailed(#[source] std::io::Error),
    #[error("serve child exited before becoming healthy (possible port conflict): {0:?}")]
    ChildExitedEarly(Option<std::process::ExitStatus>),
    #[error("serve did not become healthy within the readiness budget (spawned and waiting)")]
    ReadinessTimeout,
    #[error("serve is reachable but rejected the configured token")]
    Unauthorized,
}

/// What `ensure_serve` did to make `serve` reachable: either it was already
/// up (`External`, nothing to tear down) or this call spawned it
/// (`Spawned`, and the wrapped `Child` is killed on drop via
/// `kill_on_drop(true)`).
#[derive(Debug)]
pub enum ServeHandle {
    Spawned(tokio::process::Child),
    External,
}

const READINESS_BUDGET: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(300);

/// Ensures `ironclaw-reborn serve` is reachable at `client`'s base URL,
/// spawning it via `spawn` if it isn't already up. Returns immediately with
/// `ServeHandle::External` when a session probe already succeeds (no
/// process is started in that case). When `spawn` is `None` and the probe
/// fails, returns `SpawnError::NoServeAvailable` without waiting out the
/// readiness budget.
pub async fn ensure_serve(
    client: &ApiClient,
    spawn: Option<&ProcessInvocation>,
) -> Result<ServeHandle, SpawnError> {
    if client.session().await.is_ok() {
        return Ok(ServeHandle::External);
    }
    let invocation = spawn.ok_or(SpawnError::NoServeAvailable)?;
    let mut child = tokio::process::Command::new(&invocation.exe)
        .args(&invocation.args)
        .envs(invocation.env.iter().cloned())
        .kill_on_drop(true)
        .spawn()
        .map_err(SpawnError::SpawnFailed)?;
    poll_until_ready(client, &mut child, READINESS_BUDGET, POLL_INTERVAL).await?;
    Ok(ServeHandle::Spawned(child))
}

/// Polls `client`'s session probe every `interval` until it succeeds, the
/// child exits early, or `budget` elapses. Factored out of `ensure_serve`
/// so tests can exercise the retry/timeout/early-exit branches with a tiny
/// budget and interval instead of the real 15s/300ms constants.
pub(crate) async fn poll_until_ready(
    client: &ApiClient,
    child: &mut tokio::process::Child,
    budget: Duration,
    interval: Duration,
) -> Result<(), SpawnError> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        if let Some(status) = child.try_wait().map_err(SpawnError::SpawnFailed)? {
            return Err(SpawnError::ChildExitedEarly(Some(status)));
        }
        match client.session().await {
            Ok(_) => return Ok(()),
            Err(ClientError::Unauthorized) => return Err(SpawnError::Unauthorized),
            Err(_) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(SpawnError::ReadinessTimeout);
        }
        tokio::time::sleep(interval).await;
    }
}

// `ensure_serve`'s two happy/no-spawn paths (tests 1-2 of the plan) and the
// kill-on-drop behavior of `ServeHandle::Spawned` (test 6) exercise only
// `pub` API, so they live in `tests/spawn_ensure_serve.rs` reusing the
// shared `tests/support::MockServer` fixture. The three tests below drive
// `poll_until_ready` directly — it is `pub(crate)`, unreachable from an
// external integration-test binary — so they stay inline here with a
// minimal local axum health-check stub (axum/tower are already
// dev-dependencies of this crate for that reason).
#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::Router;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::get;
    use tokio::net::TcpListener;

    use super::*;

    /// Starts a local `/api/webchat/v2/session` stub that answers `503`
    /// for the first `fail_count` requests, then `200` forever after.
    async fn start_health_stub(fail_count: usize) -> (String, Arc<AtomicUsize>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route("/api/webchat/v2/session", get(health_handler))
            .with_state((hits.clone(), fail_count));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind health stub");
        let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("run health stub");
        });
        (base_url, hits)
    }

    async fn health_handler(
        State((hits, fail_count)): State<(Arc<AtomicUsize>, usize)>,
    ) -> (StatusCode, axum::Json<serde_json::Value>) {
        let seen = hits.fetch_add(1, Ordering::SeqCst);
        if seen < fail_count {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({"error": "not_ready"})),
            )
        } else {
            (
                StatusCode::OK,
                axum::Json(serde_json::json!({"tenant_id": "t", "user_id": "u"})),
            )
        }
    }

    /// A real but inert child the readiness poll can `try_wait()` on
    /// without it exiting mid-test.
    fn spawn_inert_child() -> tokio::process::Child {
        tokio::process::Command::new("sh")
            .args(["-c", "sleep 30"])
            .kill_on_drop(true)
            .spawn()
            .expect("spawn inert child")
    }

    #[tokio::test]
    async fn poll_until_ready_succeeds_after_n_failed_polls() {
        let (base_url, hits) = start_health_stub(2).await;
        let client = ApiClient::new(base_url, "token".to_string());
        let mut child = spawn_inert_child();

        let result = poll_until_ready(
            &client,
            &mut child,
            Duration::from_secs(5),
            Duration::from_millis(10),
        )
        .await;

        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert!(
            hits.load(Ordering::SeqCst) >= 3,
            "expected at least 3 requests, saw {}",
            hits.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn poll_until_ready_times_out_when_server_never_becomes_healthy() {
        let (base_url, _hits) = start_health_stub(usize::MAX).await;
        let client = ApiClient::new(base_url, "token".to_string());
        let mut child = spawn_inert_child();

        let result = poll_until_ready(
            &client,
            &mut child,
            Duration::from_millis(50),
            Duration::from_millis(10),
        )
        .await;

        assert!(matches!(result, Err(SpawnError::ReadinessTimeout)));
    }

    #[tokio::test]
    async fn poll_until_ready_detects_early_child_exit() {
        // No health stub needed: the child exits almost immediately, and
        // the loop checks `try_wait()` before every session probe.
        let client = ApiClient::new("http://127.0.0.1:1".to_string(), "token".to_string());
        let mut child = tokio::process::Command::new("sh")
            .args(["-c", "exit 1"])
            .kill_on_drop(true)
            .spawn()
            .expect("spawn exiting child");

        let result = poll_until_ready(
            &client,
            &mut child,
            Duration::from_secs(5),
            Duration::from_millis(10),
        )
        .await;

        match result {
            Err(SpawnError::ChildExitedEarly(Some(status))) => {
                assert!(!status.success());
            }
            other => panic!("expected ChildExitedEarly(Some(_)), got {other:?}"),
        }
    }
}
