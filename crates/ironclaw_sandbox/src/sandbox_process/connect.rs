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
//!
//! Ships unwired. `sandbox_process`'s own `connect_docker` is still the
//! single-attempt path the live transport uses; this module is re-exported at
//! the crate root for its two real future consumers in
//! `ironclaw_reborn_composition`: `sandbox_reaper_task`, which calls
//! [`connect_docker_with_retry`] before entering the reaper loop, and
//! `sandbox_composition`'s boot diagnostic, which calls
//! [`sandbox_docker_readiness`]. Repointing `connect_docker` at the retrying
//! path changes the live transport's failure timing, so it ships in the PR
//! that carries a test for it rather than riding in on this module's arrival.

#[cfg(unix)]
use std::path::PathBuf;
use std::time::Duration;

use bollard::Docker;
use ironclaw_common::env_helpers::env_or_override;

use ironclaw_host_api::process::RuntimeProcessError;

/// Env var that, when set, short-circuits Docker daemon discovery to a
/// direct connect against the given endpoint instead of probing local
/// socket candidates. Accepts a unix socket path (optionally `unix://`
/// prefixed) or an `http://host:port` / `tcp://host:port` address.
///
/// SECURITY: the non-socket form connects in **plaintext, unauthenticated**
/// — there is no `connect_with_ssl` branch here. The Docker Engine API this
/// reaches can create containers and mount host paths, so a non-loopback
/// value hands that authority to anyone on the network path. Set it only to a
/// unix socket or a loopback address (the CI/DinD case it exists for). A
/// genuinely remote daemon needs a TLS/mTLS transport, which this module does
/// not implement and must not be assumed to provide.
const DOCKER_HOST_ENV: &str = "IRONCLAW_REBORN_DOCKER_HOST";

/// Second, separately-named opt-in required before [`DOCKER_HOST_ENV`] may
/// point at a non-loopback, plaintext HTTP(S)/TCP Docker daemon.
///
/// Mirrors the `SANDBOX_ALLOW_FULL_ACCESS` pattern (`.env.example`): a
/// dangerous default (here, plaintext host-root-equivalent Docker API
/// access reachable from off-box) needs its own explicit flag rather than
/// being implied by setting the first, more innocuous-looking var. Bare
/// "reject all non-loopback" was rejected by design review: it breaks the
/// one legitimate case (a DinD/CI sidecar reachable at a container-network
/// IP, not `127.0.0.1`) and operators who hit it would just disable the
/// whole check. RFC1918-only was also rejected: a corporate `10.0.0.0/8`
/// hosts plenty of attacker footholds and is not a trust boundary.
const DOCKER_HOST_ALLOW_REMOTE_ENV: &str = "IRONCLAW_REBORN_DOCKER_HOST_ALLOW_REMOTE";

/// Maximum connect attempts before giving up.
const MAX_ATTEMPTS: u32 = 4;
/// Base backoff between attempts; doubles each retry attempt.
const BASE_BACKOFF: Duration = Duration::from_millis(250);

/// Per-attempt bound on a single connect (dial + `ping()`).
///
/// A Docker daemon that has accepted a TCP/unix-socket connection but hasn't
/// answered `ping()` within a few seconds is not healthy — waiting longer
/// than this just delays the inevitable retry (or failure). This is also the
/// client-side request timeout handed to `bollard::Docker::connect_with_*`,
/// so the two mechanisms agree on the same bound.
const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);

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
///
/// The whole attempt (all three sub-branches, sequentially) is bounded by
/// [`CONNECT_ATTEMPT_TIMEOUT`] here, in one place, so every caller — the
/// retry loop *and* the one-shot [`sandbox_docker_readiness`] probe —
/// inherits the same bound automatically. This matters most for the
/// local-default branch: `Docker::connect_with_local_defaults()` uses
/// Bollard's own internal client timeout, which this module does not
/// parameterize, so without an outer timeout here a stalled (connects but
/// never answers `ping()`) local daemon would block past what either caller
/// promises.
async fn connect_once() -> Result<Docker, RuntimeProcessError> {
    match tokio::time::timeout(CONNECT_ATTEMPT_TIMEOUT, connect_once_attempt()).await {
        Ok(result) => result,
        Err(_elapsed) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "docker connect attempt timed out after {CONNECT_ATTEMPT_TIMEOUT:?}"
        ))),
    }
}

/// The actual connect logic for a single attempt, unbounded — wrapped by
/// [`connect_once`], which is the only caller.
async fn connect_once_attempt() -> Result<Docker, RuntimeProcessError> {
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
                if let Ok(docker) = Docker::connect_with_socket(
                    &socket,
                    CONNECT_ATTEMPT_TIMEOUT.as_secs(),
                    bollard::API_DEFAULT_VERSION,
                ) && docker.ping().await.is_ok()
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
///
/// SANITIZATION: every error constructed in this function is fixed text
/// naming only the env var, the endpoint KIND (unix socket / http
/// endpoint), and a safe failure CATEGORY (connect / ping / rejected). It
/// never embeds `host` or a backend (Bollard) error's `Display`. This is
/// deliberate and applies at every branch, not just the ones that reach
/// Bollard: this crate's `AGENTS.md` bans "raw host paths, backend error
/// details ... in errors, events, snapshots, logs, or docs" outright, with
/// no carve-out for `debug!`-level logging. A previous fix here sanitized
/// only `SandboxDockerReadiness.reason`, which papered over the symptom —
/// the underlying `RuntimeProcessError` still carried the full endpoint and
/// raw Bollard error text into debug logs and every retry consumer, and an
/// `IRONCLAW_REBORN_DOCKER_HOST` containing `user:pass@host` user-info would
/// have leaked that credential into both.
///
/// This does cost operator debuggability: today nothing in this crate
/// records which literal endpoint or backend error caused a connect
/// failure, not even at debug level. That's an intentional trade rather
/// than an oversight — the operator already knows the value they put in
/// `IRONCLAW_REBORN_DOCKER_HOST`, so the software doesn't need to echo it
/// back to them, and the fixed kind+category (e.g. "unix socket ping
/// failed") is enough to tell them whether the daemon is unreachable
/// (connect) or unhealthy (ping) without re-introducing a host/credential
/// leak into a surface (debug logs, boot diagnostics) that isn't
/// necessarily trusted-internal-only.
async fn connect_override(host: &str) -> Result<Docker, RuntimeProcessError> {
    if host.starts_with("unix://") || host.starts_with('/') {
        let docker = Docker::connect_with_socket(
            host,
            CONNECT_ATTEMPT_TIMEOUT.as_secs(),
            bollard::API_DEFAULT_VERSION,
        )
        .map_err(|_e| {
            RuntimeProcessError::ExecutionFailed(format!(
                "{DOCKER_HOST_ENV} unix socket connect failed"
            ))
        })?;
        docker.ping().await.map_err(|_e| {
            RuntimeProcessError::ExecutionFailed(format!(
                "{DOCKER_HOST_ENV} unix socket ping failed"
            ))
        })?;
        return Ok(docker);
    }

    if !is_loopback_docker_host(host) && !docker_host_allow_remote() {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "{DOCKER_HOST_ENV} target is not a unix socket or loopback address; \
             plaintext access to a remote Docker Engine API is host-root-equivalent \
             (it can create containers and mount host paths). Set \
             {DOCKER_HOST_ALLOW_REMOTE_ENV}=1 to explicitly allow a non-loopback \
             Docker daemon (e.g. a DinD/CI sidecar reachable at a container-network IP)."
        )));
    }

    let docker = Docker::connect_with_http(
        host,
        CONNECT_ATTEMPT_TIMEOUT.as_secs(),
        bollard::API_DEFAULT_VERSION,
    )
    .map_err(|_e| {
        RuntimeProcessError::ExecutionFailed(format!(
            "{DOCKER_HOST_ENV} http endpoint connect failed"
        ))
    })?;
    docker.ping().await.map_err(|_e| {
        RuntimeProcessError::ExecutionFailed(format!("{DOCKER_HOST_ENV} http endpoint ping failed"))
    })?;
    Ok(docker)
}

/// Extracts the bare host (no scheme, no port, no path) from an
/// `IRONCLAW_REBORN_DOCKER_HOST` HTTP(S)/TCP address such as
/// `http://127.0.0.1:2375`, `tcp://docker-sidecar:2375`, or
/// `[::1]:2375`.
fn docker_host_component(addr: &str) -> &str {
    let without_scheme = addr.split_once("://").map_or(addr, |(_, rest)| rest);
    let without_path = without_scheme
        .split_once('/')
        .map_or(without_scheme, |(host, _)| host);
    if let Some(after_bracket) = without_path.strip_prefix('[') {
        // IPv6 literal: "[::1]:2375" -> "::1"
        return after_bracket.split(']').next().unwrap_or(after_bracket);
    }
    without_path
        .rsplit_once(':')
        .map_or(without_path, |(host, _)| host)
}

/// Whether an `IRONCLAW_REBORN_DOCKER_HOST` HTTP(S)/TCP address resolves (by
/// literal, not DNS) to loopback — the documented safe default for this
/// override. `localhost` and IPv4/IPv6 loopback literals count; a hostname
/// that merely *might* resolve to loopback at connect time does not, since
/// that would make the guard dependent on DNS state.
fn is_loopback_docker_host(addr: &str) -> bool {
    let host = docker_host_component(addr);
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Whether the operator has set the separate [`DOCKER_HOST_ALLOW_REMOTE_ENV`]
/// opt-in permitting a non-loopback [`DOCKER_HOST_ENV`] target.
fn docker_host_allow_remote() -> bool {
    env_or_override(DOCKER_HOST_ALLOW_REMOTE_ENV).is_some_and(|value| {
        let value = value.trim().to_ascii_lowercase();
        value == "1" || value == "true"
    })
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
/// backoff between attempts).
///
/// Worst-case wall-clock bound: `MAX_ATTEMPTS` (4) attempts, each capped at
/// `CONNECT_ATTEMPT_TIMEOUT` (5s), plus cumulative doubling backoff between
/// attempts (250ms + 500ms + 1000ms = 1.75s, none after the last attempt) —
/// about 21.75s, comfortably under 30s. The per-attempt bound is enforced
/// inside [`connect_once`] itself (see its doc comment), including the
/// local-default discovery branch, which otherwise uses Bollard's own
/// internal client timeout rather than a parameter this module controls.
/// `retry_worst_case_wall_clock_is_bounded` (below) pins this arithmetic so a
/// future change to the constants can't silently blow the bound back out.
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
/// gate that blocks boot waiting for the daemon to come up). Bounded by
/// [`CONNECT_ATTEMPT_TIMEOUT`] via [`connect_once`] — a daemon that accepts
/// the connection but never answers cannot stall this past that bound.
pub async fn sandbox_docker_readiness() -> SandboxDockerReadiness {
    match connect_once().await {
        Ok(_) => SandboxDockerReadiness::Ready,
        Err(err) => {
            // `err` is already sanitized at construction time
            // (`connect_once`/`connect_override`): fixed endpoint-kind +
            // failure-category text only, never the configured socket path,
            // remote address, or Bollard's raw backend error. That holds
            // for this `debug!` field too, not just the public `reason`
            // below — this probe is documented as a boot diagnostic
            // surfaced on "a startup log line or health endpoint", a
            // surface that is not necessarily trusted-internal-only, and
            // this crate's `AGENTS.md` bans raw host paths/backend error
            // details in logs outright (no debug-level carve-out).
            tracing::debug!(error = %err, "sandbox docker readiness probe failed");
            SandboxDockerReadiness::Unreachable {
                reason: "docker daemon unreachable".to_string(),
            }
        }
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

    // The override has two branches; the test above only reaches the unix
    // socket one. This reaches the http/tcp one, which is the branch CI
    // runners and DinD actually take, and pins that its failure is still
    // attributed to the env var rather than falling through to the generic
    // local-discovery message. Port 1 on loopback has nothing listening.
    #[test]
    fn docker_host_env_override_http_branch_reports_the_env_var() {
        let _guard = lock_env();
        set_runtime_env(DOCKER_HOST_ENV, "http://127.0.0.1:1");

        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime for test")
            .block_on(connect_once());

        remove_runtime_env(DOCKER_HOST_ENV);

        let err = result.expect_err("nothing is listening on 127.0.0.1:1");
        let message = err.to_string();
        assert!(
            message.contains(DOCKER_HOST_ENV),
            "expected http override branch error to name {DOCKER_HOST_ENV}, got: {message}"
        );
    }

    // Plaintext, unauthenticated access to a remote Docker Engine API is
    // host-root-equivalent (see the module doc comment on DOCKER_HOST_ENV).
    // A non-loopback host must be refused by default, before any socket is
    // even opened, unless the operator has separately opted in via
    // DOCKER_HOST_ALLOW_REMOTE_ENV. This reaches the guard directly (not
    // through connect_once/connect_docker_with_retry) so it doesn't need a
    // reachable daemon: the trust-boundary check must fire before any I/O.
    #[test]
    fn connect_override_rejects_non_loopback_host_without_opt_in() {
        let _guard = lock_env();
        remove_runtime_env(DOCKER_HOST_ALLOW_REMOTE_ENV);

        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime for test")
            .block_on(connect_override("http://203.0.113.5:2375"));

        let err = result.expect_err("non-loopback host must be rejected without the opt-in");
        let message = err.to_string();
        assert!(
            message.contains(DOCKER_HOST_ALLOW_REMOTE_ENV),
            "error must name the opt-in env var so an operator doesn't have \
             to read Rust source to proceed, got: {message}"
        );
        // ironloop review finding (PR #6746): this rejection is constructed
        // from the operator-supplied host without ever touching Bollard, so
        // it's easy to assume echoing it back is harmless — but the same
        // host string can carry `user:pass@` credentials, and this crate's
        // `AGENTS.md` bans raw host paths in errors outright. The host must
        // NOT appear in the message.
        assert!(
            !message.contains("203.0.113.5"),
            "error must not echo the disallowed host back (raw host paths \
             are banned from errors — see AGENTS.md), got: {message}"
        );
    }

    // With the opt-in set, the trust-boundary check must get out of the way
    // — the call should reach real Docker connect/ping logic (which then
    // fails because nothing is listening at that address, not because of
    // the trust boundary). This machine has no Docker daemon, so this only
    // proves the guard was passed, not that a real remote daemon connects;
    // see the module doc comment for why a Docker-client mock was rejected.
    #[test]
    fn connect_override_proceeds_past_trust_boundary_with_opt_in() {
        let _guard = lock_env();
        set_runtime_env(DOCKER_HOST_ALLOW_REMOTE_ENV, "1");

        // User-info makes the policy parser classify this as non-loopback,
        // while the actual connection remains on a closed loopback port so
        // the test does not violate the hermetic test boundary.
        let host = "http://remote-test@127.0.0.1:1";
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime for test")
            .block_on(connect_override(host));

        remove_runtime_env(DOCKER_HOST_ALLOW_REMOTE_ENV);

        let err = result.expect_err("127.0.0.1:1 has nothing listening");
        let message = err.to_string();
        assert!(
            !message.contains(DOCKER_HOST_ALLOW_REMOTE_ENV),
            "with the opt-in set, the failure must come from the real \
             connect/ping attempt, not the trust-boundary guard, got: {message}"
        );
    }

    // Loopback addresses must be unaffected by the new guard — this is the
    // documented safe default and must keep working with no opt-in set.
    #[test]
    fn connect_override_loopback_host_is_unaffected_by_the_remote_guard() {
        let _guard = lock_env();
        remove_runtime_env(DOCKER_HOST_ALLOW_REMOTE_ENV);

        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime for test")
            .block_on(connect_override("http://127.0.0.1:1"));

        let err = result.expect_err("nothing is listening on 127.0.0.1:1");
        let message = err.to_string();
        assert!(
            !message.contains(DOCKER_HOST_ALLOW_REMOTE_ENV),
            "loopback host must not be routed through the remote-host guard, \
             got: {message}"
        );
    }

    // Uses the env override (like the two tests above) so the daemon is
    // deterministically unreachable and the underlying `RuntimeProcessError`
    // is guaranteed to embed the configured endpoint (see
    // `connect_override`'s format! strings) — that's the exact string the
    // public `reason` must NOT leak. See the comment on
    // `docker_host_env_override_is_consulted_first` for why this is a plain
    // `#[test]` driving the async probe via `block_on` rather than
    // `#[tokio::test]` holding the guard across an `.await`.
    #[test]
    fn readiness_surfaces_reason_on_unreachable_daemon() {
        let _guard = lock_env();
        let unreachable_host = "/nonexistent/ironclaw-test-docker-readiness.sock";
        set_runtime_env(DOCKER_HOST_ENV, unreachable_host);

        let readiness = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime for test")
            .block_on(sandbox_docker_readiness());

        remove_runtime_env(DOCKER_HOST_ENV);

        match readiness {
            SandboxDockerReadiness::Unreachable { reason } => {
                assert!(!reason.is_empty(), "reason should be a non-empty string");
                assert!(
                    !reason.contains(unreachable_host),
                    "readiness reason leaked the configured Docker endpoint \
                     into a diagnostic surface that isn't necessarily \
                     trusted-internal-only, got: {reason}"
                );
            }
            SandboxDockerReadiness::Ready => {
                panic!(
                    "expected Unreachable: {unreachable_host} is a nonexistent \
                     socket path and can never be Ready"
                );
            }
        }
    }

    // Pins the worst-case wall-clock bound documented on
    // `connect_docker_with_retry`. A real end-to-end timing test would need
    // a Docker daemon that accepts the connection but never answers
    // `ping()` (not reproducible without a mock Docker client, which was
    // rejected by design review for this module) and would be flaky under
    // CI scheduling jitter regardless. Pinning the constants' arithmetic
    // instead makes any future change to MAX_ATTEMPTS, CONNECT_ATTEMPT_TIMEOUT,
    // or BASE_BACKOFF that blows the documented bound back out fail loudly
    // and deterministically.
    #[test]
    fn retry_worst_case_wall_clock_is_bounded() {
        let mut worst_case = Duration::ZERO;
        let mut backoff = BASE_BACKOFF;
        for attempt in 0..MAX_ATTEMPTS {
            worst_case += CONNECT_ATTEMPT_TIMEOUT;
            if attempt + 1 < MAX_ATTEMPTS {
                worst_case += backoff;
                backoff *= 2;
            }
        }

        assert_eq!(
            worst_case,
            Duration::from_millis(21_750),
            "connect_docker_with_retry's worst-case wall clock changed; if \
             intentional, update this pin AND the doc comment on \
             connect_docker_with_retry",
        );
        assert!(
            worst_case <= Duration::from_secs(30),
            "connect_docker_with_retry's worst-case wall clock ({worst_case:?}) \
             no longer matches its documented \"about 21.75s, comfortably \
             under 30s\" bound",
        );
    }

    // ironloop review finding (PR #6746): sandbox_docker_readiness() called
    // connect_once() directly with nothing wrapping the local-default
    // discovery branch, which uses Bollard's own internal 120s client
    // timeout rather than CONNECT_ATTEMPT_TIMEOUT. A daemon that accepts the
    // connection but never answers `ping()` (the dangerous case — a closed
    // port fails fast and proves nothing) could stall the "cheap one-shot
    // probe" documented on `SandboxDockerReadiness` for up to two minutes.
    //
    // This reproduces that exact daemon shape: a real unix-socket listener
    // that accepts the connection and then never writes a response. Real
    // `DOCKER_HOST` (Bollard's own env var, not this module's
    // `IRONCLAW_REBORN_DOCKER_HOST` override) is pointed at it so
    // `connect_once` takes the `Docker::connect_with_local_defaults()`
    // branch — the one with no parameterized timeout — rather than the
    // override branch, which already threaded `CONNECT_ATTEMPT_TIMEOUT`
    // through to Bollard before this fix.
    //
    // Plain `#[test]` (not `#[tokio::test]`), same reasoning as
    // `docker_host_env_override_is_consulted_first` above: `lock_env()`'s
    // guard must not be held across an `.await` in an outer async fn
    // (clippy `await_holding_lock`), so a current-thread runtime's
    // `block_on` drives the probe to completion synchronously inside the
    // guarded section instead. `block_on` also drives the
    // `tokio::time::timeout` wrapper, so the ceiling assertion still holds.
    #[cfg(unix)]
    #[test]
    fn sandbox_docker_readiness_bounded_when_daemon_accepts_but_never_answers() {
        let _guard = lock_env();
        remove_runtime_env(DOCKER_HOST_ENV);

        // Unix socket paths are capped at `SUN_LEN` (~104 bytes on macOS),
        // which `std::env::temp_dir()` can blow through on some CI/dev
        // layouts — use `/tmp` directly, kept short.
        let dir = std::path::PathBuf::from("/tmp").join(format!(
            "ic-dsock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                % 1_000_000
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir for stalled docker socket");
        let socket_path = dir.join("d.sock");

        let listener = std::os::unix::net::UnixListener::bind(&socket_path)
            .expect("bind stalled docker socket");
        // Detached on purpose: the process exits when the test binary's main
        // thread returns, which tears this down regardless of whether the
        // 300s sleep below ever completes.
        let _accept_thread = std::thread::spawn(move || {
            if let Ok((stream, _addr)) = listener.accept() {
                std::thread::sleep(Duration::from_secs(300));
                drop(stream);
            }
        });

        let original_docker_host = std::env::var_os("DOCKER_HOST");
        // SAFETY: guarded by `lock_env()`, same pattern as
        // `runtime_mask_hides_real_env` above — no other test mutates
        // `DOCKER_HOST` concurrently.
        unsafe {
            std::env::set_var("DOCKER_HOST", format!("unix://{}", socket_path.display()));
        }

        // Generous ceiling, not a tight window: comfortably more than
        // 2x CONNECT_ATTEMPT_TIMEOUT so ordinary CI scheduling jitter can't
        // flake it, but far short of Bollard's 120s internal default —
        // enough to prove the bound without a slow test.
        let ceiling = CONNECT_ATTEMPT_TIMEOUT * 2 + Duration::from_secs(2);
        let started = std::time::Instant::now();
        let outcome = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime for test")
            .block_on(async { tokio::time::timeout(ceiling, sandbox_docker_readiness()).await });
        let elapsed = started.elapsed();

        // SAFETY: still under the same `lock_env()` guard as the set above.
        unsafe {
            match original_docker_host {
                Some(value) => std::env::set_var("DOCKER_HOST", value),
                None => std::env::remove_var("DOCKER_HOST"),
            }
        }
        remove_runtime_env(DOCKER_HOST_ENV);
        std::fs::remove_dir_all(&dir).ok();

        let readiness = outcome.unwrap_or_else(|_| {
            panic!(
                "sandbox_docker_readiness() did not return within {ceiling:?} \
                 (2x CONNECT_ATTEMPT_TIMEOUT + slack) even though it is \
                 documented as a cheap, bounded one-shot probe; a daemon \
                 that accepts the connection and never answers must not be \
                 able to stall it past that bound (elapsed={elapsed:?})"
            )
        });

        assert_eq!(
            readiness,
            SandboxDockerReadiness::Unreachable {
                reason: "docker daemon unreachable".to_string(),
            },
            "expected the stalled daemon to be reported Unreachable, not Ready"
        );
    }

    // ironloop review finding (PR #6746): the earlier fix in this file
    // (`readiness_surfaces_reason_on_unreachable_daemon`) only sanitized
    // `SandboxDockerReadiness.reason`. The underlying `RuntimeProcessError`
    // built by `connect_override` still embedded the full configured
    // endpoint and Bollard's raw error text, so every *other* consumer of
    // that error (debug logs, the retry loop's exhausted error) still saw
    // it — and if `IRONCLAW_REBORN_DOCKER_HOST` carried `user:pass@host`
    // user-info, that credential leaked into both. This pins the fix at
    // its source (`connect_override`'s error constructors), covering the
    // http connect/ping branch where the credential-bearing case actually
    // lives.
    //
    // `readiness`'s `debug!(error = %err, ...)` field logs `err`'s exact
    // `Display` output — the same string asserted on below — so proving
    // this string is clean also proves the log field is clean; there is no
    // separate log-formatting path to test independently.
    //
    // RED evidence: reverting the `_e` param names in `connect_override`
    // back to `e` and re-inserting `for {host}: {e}` into the four
    // Bollard-facing format! calls (the pre-fix shape) makes this test fail
    // with the host and "hunter2" both present in the message — see the
    // task report for the captured failure output.
    #[test]
    fn connect_override_http_errors_never_leak_host_or_userinfo_credential() {
        let _guard = lock_env();

        // `docker_host_component` parses the host out by splitting on the
        // last `:`, so a `user:pass@host` URL's parsed "host" is
        // `user:pass@host` — never equal to `localhost` nor a valid
        // `IpAddr` — and `is_loopback_docker_host` (correctly, fail-closed)
        // treats any user-info-bearing URL as non-loopback regardless of
        // the real host. Set the remote opt-in so this test reaches the
        // connect/ping error branches under test rather than the separate
        // rejection branch (covered by
        // `connect_override_rejects_non_loopback_host_without_opt_in`).
        set_runtime_env(DOCKER_HOST_ALLOW_REMOTE_ENV, "1");
        let host = "http://user:hunter2@127.0.0.1:1";

        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime for test")
            .block_on(connect_override(host));

        remove_runtime_env(DOCKER_HOST_ALLOW_REMOTE_ENV);

        let err = result.expect_err("nothing is listening on 127.0.0.1:1");
        let message = err.to_string();

        assert!(
            !message.contains("hunter2"),
            "error must never leak a credential supplied via user-info in \
             {DOCKER_HOST_ENV}, got: {message}"
        );
        assert!(
            !message.contains("user:hunter2"),
            "error must never leak the user-info component of {DOCKER_HOST_ENV}, \
             got: {message}"
        );
        assert!(
            !message.contains("127.0.0.1"),
            "error must not embed the configured host at all (fixed kind + \
             category only), got: {message}"
        );
        assert!(
            message.contains(DOCKER_HOST_ENV) && message.contains("http endpoint"),
            "error should still name the env var and endpoint kind so an \
             operator gets a safe category, got: {message}"
        );
    }

    // Companion to the http-branch test above, covering the unix socket
    // connect/ping error branch with a path that looks like it could carry
    // sensitive topology (a nonstandard, identifying socket path).
    #[test]
    fn connect_override_unix_socket_errors_never_leak_the_configured_path() {
        let _guard = lock_env();

        let host = "/nonexistent/ironclaw-test-docker-leak-check.sock";

        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime for test")
            .block_on(connect_override(host));

        let err = result.expect_err("no daemon reachable at nonexistent override path");
        let message = err.to_string();

        assert!(
            !message.contains(host),
            "error must not embed the configured unix socket path, got: {message}"
        );
        assert!(
            message.contains(DOCKER_HOST_ENV) && message.contains("unix socket"),
            "error should still name the env var and endpoint kind so an \
             operator gets a safe category, got: {message}"
        );
    }
}
