//! Docker daemon connectivity hardening for the Reborn sandbox process
//! transport.
//!
//! Docker socket discovery is inherently flaky in dev environments (Docker
//! Desktop restarts, Colima cold start, transient daemon busy states, etc).
//! This module wraps the existing single-attempt connect logic
//! ([`connect_once`]) in a bounded retry loop, adds an
//! `IRONCLAW_REBORN_DOCKER_HOST` env override for environments where local
//! socket discovery doesn't apply (CI runners, remote daemons), and exposes
//! a cheap readiness probe for boot diagnostics.
//!
//! CRITICAL: retry exhaustion here always propagates as a hard
//! [`RuntimeProcessError`]. Callers MUST NOT catch that error and fall back
//! to running the command unsandboxed on the host — there is no
//! host-execution fallback path for sandboxed command execution. See
//! `docs/safety-and-sandbox.md`.

#[cfg(unix)]
use std::path::PathBuf;
use std::time::Duration;

use bollard::Docker;
use ironclaw_common::env_helpers::env_or_override;

use crate::RuntimeProcessError;

/// Env var that, when set, short-circuits Docker daemon discovery to a
/// direct connect against the given endpoint instead of probing local
/// socket candidates. Accepts a unix socket path (optionally `unix://`
/// prefixed) or an `http://host:port` / `tcp://host:port` address.
const DOCKER_HOST_ENV: &str = "IRONCLAW_REBORN_DOCKER_HOST";

/// Maximum connect attempts before giving up.
const MAX_ATTEMPTS: u32 = 4;
/// Base backoff between attempts; doubles each retry attempt.
const BASE_BACKOFF: Duration = Duration::from_millis(250);

/// Outcome of a Docker daemon readiness probe, surfaced as a boot
/// diagnostic.
///
/// This is a diagnostic signal only (e.g. for a startup log line or health
/// endpoint) — it must never be used to gate a fallback to unsandboxed
/// execution. See module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxDockerReadiness {
    Ready,
    Unreachable { reason: String },
}

/// Single connection attempt: env override first, then local-default
/// discovery, then well-known unix socket candidates. No retry here — see
/// [`connect_docker_with_retry`] for the retrying entrypoint.
async fn connect_once() -> Result<Docker, RuntimeProcessError> {
    if let Some(override_host) = env_or_override(DOCKER_HOST_ENV) {
        return connect_override(&override_host).await;
    }

    if let Ok(docker) = Docker::connect_with_local_defaults()
        && docker.ping().await.is_ok()
    {
        return Ok(docker);
    }

    #[cfg(unix)]
    {
        for socket in unix_socket_candidates() {
            if socket.exists() {
                let socket = socket.to_string_lossy();
                if let Ok(docker) =
                    Docker::connect_with_socket(&socket, 120, bollard::API_DEFAULT_VERSION)
                    && docker.ping().await.is_ok()
                {
                    return Ok(docker);
                }
            }
        }
    }

    Err(RuntimeProcessError::ExecutionFailed(
        "could not connect to Docker daemon for Reborn sandbox".to_string(),
    ))
}

/// Connect to the daemon at an explicit `IRONCLAW_REBORN_DOCKER_HOST`
/// override: tried as a unix socket path first (when it looks like one),
/// otherwise as an HTTP(S) address.
async fn connect_override(host: &str) -> Result<Docker, RuntimeProcessError> {
    if host.starts_with("unix://") || host.starts_with('/') {
        let docker =
            Docker::connect_with_socket(host, 120, bollard::API_DEFAULT_VERSION).map_err(|e| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "{DOCKER_HOST_ENV} unix socket connect failed for {host}: {e}"
                ))
            })?;
        docker.ping().await.map_err(|e| {
            RuntimeProcessError::ExecutionFailed(format!(
                "{DOCKER_HOST_ENV} unix socket ping failed for {host}: {e}"
            ))
        })?;
        return Ok(docker);
    }

    let docker =
        Docker::connect_with_http(host, 120, bollard::API_DEFAULT_VERSION).map_err(|e| {
            RuntimeProcessError::ExecutionFailed(format!(
                "{DOCKER_HOST_ENV} http connect failed for {host}: {e}"
            ))
        })?;
    docker.ping().await.map_err(|e| {
        RuntimeProcessError::ExecutionFailed(format!(
            "{DOCKER_HOST_ENV} http ping failed for {host}: {e}"
        ))
    })?;
    Ok(docker)
}

#[cfg(unix)]
fn unix_socket_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        candidates.push(home.join(".docker/run/docker.sock"));
        candidates.push(home.join(".colima/default/docker.sock"));
        candidates.push(home.join(".rd/docker.sock"));
    }
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        candidates.push(runtime_dir.join("docker.sock"));
    }
    candidates
}

/// Retry `f` up to `attempts` times with doubling backoff starting at
/// `base_backoff`, sleeping between attempts (not after the last one).
///
/// Kept private and local to this module: there is exactly one production
/// caller ([`connect_docker_with_retry`]); this is not a general-purpose
/// retry utility for the crate.
async fn run_with_retry<F, Fut, T>(
    attempts: u32,
    base_backoff: Duration,
    mut f: F,
) -> Result<T, RuntimeProcessError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, RuntimeProcessError>>,
{
    let mut last_err = None;
    let mut backoff = base_backoff;

    for attempt in 0..attempts {
        match f().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                last_err = Some(err);
                if attempt + 1 < attempts {
                    tokio::time::sleep(backoff).await;
                    backoff *= 2;
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        RuntimeProcessError::ExecutionFailed(
            "could not connect to Docker daemon for Reborn sandbox".to_string(),
        )
    }))
}

/// Connect to the Docker daemon with a bounded retry loop (exponential
/// backoff, capped total wait of a few seconds).
///
/// CRITICAL: on retry exhaustion this returns `Err`, which callers MUST
/// propagate as a hard failure. There is no fallback to running the sandbox
/// command unsandboxed on the host — see module docs and
/// `docs/safety-and-sandbox.md`.
pub async fn connect_docker_with_retry() -> Result<Docker, RuntimeProcessError> {
    run_with_retry(MAX_ATTEMPTS, BASE_BACKOFF, connect_once).await
}

/// Boot-time Docker daemon readiness probe.
///
/// Thin wrapper around a single connect attempt (not the retry loop — this
/// is meant to be a fast, one-shot diagnostic reported at startup, not a
/// gate that blocks boot waiting for the daemon to come up).
pub async fn sandbox_docker_readiness() -> SandboxDockerReadiness {
    match connect_once().await {
        Ok(_) => SandboxDockerReadiness::Ready,
        Err(err) => SandboxDockerReadiness::Unreachable {
            reason: err.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_common::env_helpers::{lock_env, remove_runtime_env, set_runtime_env};

    use super::*;

    #[tokio::test]
    async fn retry_loop_succeeds_after_transient_failures() {
        let calls = AtomicUsize::new(0);

        let result: Result<u32, RuntimeProcessError> =
            run_with_retry(5, Duration::from_millis(1), || {
                let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
                async move {
                    if call < 3 {
                        Err(RuntimeProcessError::ExecutionFailed(format!(
                            "transient failure {call}"
                        )))
                    } else {
                        Ok(42)
                    }
                }
            })
            .await;

        assert_eq!(result, Ok(42));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_loop_exhausts_and_returns_last_error() {
        let calls = AtomicUsize::new(0);

        let result: Result<u32, RuntimeProcessError> =
            run_with_retry(4, Duration::from_millis(1), || {
                let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
                async move {
                    Err::<u32, _>(RuntimeProcessError::ExecutionFailed(format!(
                        "permanent failure {call}"
                    )))
                }
            })
            .await;

        assert_eq!(calls.load(Ordering::SeqCst), 4);
        match result {
            Err(RuntimeProcessError::ExecutionFailed(msg)) => {
                assert_eq!(msg, "permanent failure 4");
            }
            other => panic!("expected exhausted ExecutionFailed, got {other:?}"),
        }
    }

    // Live daemon behavior for the override branch (actually dialing the
    // configured endpoint) is proven in the Docker-gated integration tier —
    // this machine has no Docker daemon. This test only proves the env
    // override is read and selected before local-default discovery: an
    // unreachable override path must fail with an error naming the override
    // env var, not the generic local-discovery failure message.
    //
    // Plain `#[test]` (not `#[tokio::test]`) so the `lock_env()` guard —
    // which must stay held for the whole set-env/connect/read-error window
    // to keep this test's env mutation from interleaving with any other
    // test touching the same runtime-env overlay — is never held across a
    // `.await` in an outer async fn. `block_on` drives `connect_once` to
    // completion synchronously inside the guarded section instead, so
    // there's no clippy-visible suspension point while the guard is live.
    #[test]
    fn docker_host_env_override_is_consulted_first() {
        let _guard = lock_env();
        set_runtime_env(DOCKER_HOST_ENV, "/nonexistent/ironclaw-test-docker.sock");

        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime for test")
            .block_on(connect_once());

        remove_runtime_env(DOCKER_HOST_ENV);

        let err = result.expect_err("no daemon reachable at nonexistent override path");
        let message = err.to_string();
        assert!(
            message.contains(DOCKER_HOST_ENV),
            "expected override branch error to name {DOCKER_HOST_ENV}, got: {message}"
        );
    }

    // Natural on this machine: no Docker daemon is running, so connect_once
    // (and therefore the readiness probe) fails through local discovery.
    // See the comment on `docker_host_env_override_is_consulted_first` for
    // why this is a plain `#[test]` driving the async probe via `block_on`
    // rather than `#[tokio::test]` holding the guard across an `.await`.
    #[test]
    fn readiness_surfaces_reason_on_unreachable_daemon() {
        let _guard = lock_env();
        remove_runtime_env(DOCKER_HOST_ENV);

        let readiness = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime for test")
            .block_on(sandbox_docker_readiness());

        match readiness {
            SandboxDockerReadiness::Unreachable { reason } => {
                assert!(!reason.is_empty(), "reason should be a non-empty string");
            }
            SandboxDockerReadiness::Ready => {
                // A real Docker daemon happens to be reachable on this
                // machine/CI runner; readiness reporting Ready is also a
                // valid, non-flaky outcome for this probe.
            }
        }
    }
}
