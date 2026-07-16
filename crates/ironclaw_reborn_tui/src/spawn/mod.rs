//! Launches `ironclaw-reborn serve` when it isn't already reachable, and
//! polls the launched (or already-running) process until it answers
//! `/api/webchat/v2/session`. [`ensure_serve`] is the entry point `lib.rs`'s
//! `run_tui` calls before starting the terminal event loop; the readiness
//! poll itself is factored into `poll_until_ready` (`pub(crate)`,
//! parameterized by `Duration`) so tests can drive it with a tiny budget
//! and interval instead of waiting out the real 15s/300ms constants.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::client::{ApiClient, ClientError};

/// Name of the env var carrying the resolved Reborn home directory, mirrored
/// from `ironclaw_reborn_config::REBORN_HOME_ENV`. Kept as a literal here
/// rather than adding a dependency on `ironclaw_reborn_config` for one
/// constant: this crate never resolves Reborn env vars itself (see the
/// module doc), it only reads what `ironclaw_reborn_cli::serve_invocation`
/// already put in `ProcessInvocation.env`.
const REBORN_HOME_ENV_KEY: &str = "IRONCLAW_REBORN_HOME";

/// Log file name the spawned serve child's stdout/stderr append to, under
/// the Reborn home resolved from `invocation.env`.
const SERVE_LOG_FILE_NAME: &str = "tui-serve.log";

/// Resolves where the spawned serve child's stdout/stderr log should live:
/// `<home>/tui-serve.log` when `invocation.env` carries the Reborn home,
/// `None` when it doesn't (the caller falls back to `Stdio::null()` in that
/// case). Pure and side-effect-free so it's unit-testable without touching
/// the filesystem or spawning a process.
fn serve_log_path(invocation: &ProcessInvocation) -> Option<PathBuf> {
    invocation
        .env
        .iter()
        .find(|(key, _)| key == REBORN_HOME_ENV_KEY)
        .map(|(_, home)| PathBuf::from(home).join(SERVE_LOG_FILE_NAME))
}

/// Opens `path` (create/append) independently for stdout and stderr so the
/// spawned serve child's log output never inherits the TUI's terminal —
/// without this, `IRONCLAW_REBORN_LOG` output bleeds into ratatui's
/// raw-mode alternate screen and corrupts the UI. Falls back to
/// `Stdio::null()` per-stream on an open error rather than failing the
/// spawn.
fn open_log_stdio(path: &Path) -> (Stdio, Stdio) {
    let open_one = || {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|err| format!("failed to open tui serve log {}: {err}", path.display()))
    };
    let stdout = open_one()
        .map(Stdio::from)
        .unwrap_or_else(|_reason| Stdio::null());
    let stderr = open_one()
        .map(Stdio::from)
        .unwrap_or_else(|_reason| Stdio::null());
    (stdout, stderr)
}

/// Decides the `Stdio` targets for the spawned serve child's stdout/stderr:
/// a shared log file under the Reborn home when it's available, otherwise
/// `Stdio::null()` for both.
fn child_stdio(invocation: &ProcessInvocation) -> (Stdio, Stdio) {
    match serve_log_path(invocation) {
        Some(path) => open_log_stdio(&path),
        None => (Stdio::null(), Stdio::null()),
    }
}

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
    let (stdout, stderr) = child_stdio(invocation);
    let mut child = tokio::process::Command::new(&invocation.exe)
        .args(&invocation.args)
        .envs(invocation.env.iter().cloned())
        .kill_on_drop(true)
        .stdout(stdout)
        .stderr(stderr)
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

    // The stdio-redirection fix (spawned serve child's stdout/stderr must
    // never inherit the TUI's terminal — see the module-level fix note)
    // isn't testable end-to-end without spawning a real, flaky process, so
    // these tests target the pure decision helper (`serve_log_path`) and
    // the file-open fallback (`child_stdio`) directly.

    fn invocation_with_env(env: Vec<(&str, &str)>) -> ProcessInvocation {
        ProcessInvocation {
            exe: PathBuf::from("ironclaw-reborn"),
            args: vec!["serve".to_string()],
            env: env
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn serve_log_path_uses_home_from_invocation_env() {
        let invocation = invocation_with_env(vec![(REBORN_HOME_ENV_KEY, "/tmp/some-reborn-home")]);

        assert_eq!(
            serve_log_path(&invocation),
            Some(PathBuf::from("/tmp/some-reborn-home/tui-serve.log"))
        );
    }

    #[test]
    fn serve_log_path_is_none_without_home_env() {
        let invocation = invocation_with_env(vec![("SOME_OTHER_VAR", "value")]);

        assert_eq!(serve_log_path(&invocation), None);
    }

    #[test]
    fn child_stdio_creates_log_file_under_resolved_home() {
        let home =
            std::env::temp_dir().join(format!("ironclaw-tui-stdio-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).expect("create test home");
        let invocation = invocation_with_env(vec![(
            REBORN_HOME_ENV_KEY,
            home.to_str().expect("temp home path is valid utf-8"),
        )]);

        let (_stdout, _stderr) = child_stdio(&invocation);

        assert!(
            home.join(SERVE_LOG_FILE_NAME).exists(),
            "expected {} to be created under {}",
            SERVE_LOG_FILE_NAME,
            home.display()
        );

        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn child_stdio_does_not_panic_without_home_env() {
        let invocation = invocation_with_env(vec![]);

        // Nothing observable to assert on `Stdio::null()` itself (it's
        // opaque); the meaningful assertion is that resolving stdio for a
        // spawn-invocation lacking a Reborn home falls back cleanly.
        let (_stdout, _stderr) = child_stdio(&invocation);
    }

    #[test]
    fn child_stdio_falls_back_to_null_when_log_file_cannot_be_opened() {
        // The log file's parent directory doesn't exist and `OpenOptions`
        // won't create it, so both opens fail; the fallback path must not
        // panic, and no directory should get created as a side effect.
        let missing_parent = std::env::temp_dir().join(format!(
            "ironclaw-tui-stdio-missing-{}",
            uuid::Uuid::new_v4()
        ));
        let invocation = invocation_with_env(vec![(
            REBORN_HOME_ENV_KEY,
            missing_parent
                .to_str()
                .expect("temp home path is valid utf-8"),
        )]);

        let (_stdout, _stderr) = child_stdio(&invocation);

        assert!(
            !missing_parent.exists(),
            "no directory should be created for an unopenable log path"
        );
    }
}
