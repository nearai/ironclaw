//! Remote-request timeout resolution and the pinned outbound HTTP client
//! shared by every Trace Commons call.

use std::time::Duration;

use crate::contribution::*;

#[cfg(test)]
tokio::task_local! {
    /// Test-only, task-scoped override for the remote-request timeout.
    ///
    /// The timeout was historically configured for tests by setting the
    /// process-global `IRONCLAW_TRACE_REMOTE_REQUEST_TIMEOUT_MS` env var via a
    /// `set_var`/`remove_var` guard. That is a process-global mutation: under
    /// parallel test execution, the short (e.g. 50ms) value set by one timing
    /// test leaked into every other test that built a trace HTTP client on
    /// another thread, causing spurious `operation timed out` failures against
    /// fast local mock servers (and the reverse: the guard's `remove_var`
    /// reverting an in-flight request to the 30s default). A task-local
    /// override is visible only within the awaiting test's own task tree —
    /// `trace_remote_http_client` is called from the same task that runs the
    /// submit `.await` — so it is fully isolated across parallel tests with no
    /// process-global state and no change to production behavior.
    ///
    /// CAVEAT: the override only propagates within the awaiting task's tree. If
    /// a future refactor wraps the HTTP call in `tokio::spawn` (a new task that
    /// does not inherit task-locals), the spawned request would silently bypass
    /// this override and fall back to the env/default timeout.
    pub(crate) static TEST_REMOTE_REQUEST_TIMEOUT_OVERRIDE: Duration;
}

pub(crate) fn trace_remote_request_timeout() -> Duration {
    // Test-only, task-scoped override takes precedence (see task-local docs).
    #[cfg(test)]
    if let Ok(override_timeout) = TEST_REMOTE_REQUEST_TIMEOUT_OVERRIDE.try_with(|t| *t) {
        return override_timeout;
    }
    std::env::var(TRACE_REMOTE_REQUEST_TIMEOUT_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(TRACE_REMOTE_REQUEST_DEFAULT_TIMEOUT_MS))
}

// Justification — why the background trace-upload / status-sync lane does NOT
// route through host `RuntimeHttpEgress` (unlike the agent-invoked onboard /
// profile_token / profile_set paths, which do):
//
// The host egress pipeline exists to gate *model-driven* external writes — it
// attaches a per-request capability identity + resource scope, an approval-gate
// obligation, and a host-derived credential-injection plan. The trace queue
// flush/sync worker has none of those by construction: it is a durable runtime
// task (`spawn_trace_queue_flush_worker`) that drains the local contribution
// queue for many scopes on a fixed interval, with no model input, no
// per-request capability, and no approval gate. Forcing it through egress would
// require synthesizing a fake capability id + scope and a credential model for a
// gate-less task — added complexity with no security benefit, because this lane
// already: (1) sends only trace envelopes that passed the safety/redaction
// pipeline at capture time (scan-before-storage); (2) targets the user's own
// operator-enrolled ingest endpoint, validated for SSRF/private-IP via
// `validate_trace_commons_ingest_url` with pinned `resolve_to_addrs`; and
// (3) authenticates with the enrolled-policy bearer token, never a model-
// supplied value. So this is an intentional trusted internal lane, not an
// un-gated external-write hole. See PR #4559 discussion.
// In addition to the enrollment-time endpoint validation described above, each
// background request pins its own DNS resolution below
// (`pinned_trace_remote_http_client`), so a host that passed validation at
// enrollment cannot later rebind to a private/internal address and receive the
// bearer-authenticated submit/status/revoke requests.
pub(crate) async fn pinned_trace_remote_http_client(
    endpoint: &str,
) -> Result<reqwest::Client, TraceRemoteRequestFailure> {
    let url = reqwest::Url::parse(endpoint).map_err(|error| {
        TraceRemoteRequestFailure::endpoint_invalid(format!(
            "trace remote endpoint is not a valid URL: {error}"
        ))
    })?;
    // Every request built here carries the enrolled bearer token, so the
    // endpoint has to be TLS (or literal loopback for standalone). The comment
    // above claimed this lane was "validated ... via
    // `validate_trace_commons_ingest_url`", but nothing on the
    // submit/status/revoke path ever called it — that validator only ran from
    // `community_profile_url_from_policy`. Meanwhile `ironclaw traces opt-in
    // --endpoint <url>` writes `policy.ingestion_endpoint` unvalidated, so
    // `--endpoint http://public-host/...` plus a bearer shipped the token in
    // clear text (#7144). Validating in the builder makes the claim true: this
    // is what attaches the credential, so this is where it fails closed.
    validate_trace_commons_ingest_url(&url).map_err(|error| {
        TraceRemoteRequestFailure::endpoint_invalid(format!(
            "trace remote endpoint rejected: {error}"
        ))
    })?;
    let host = url
        .host_str()
        .ok_or_else(|| {
            TraceRemoteRequestFailure::endpoint_invalid(
                "trace remote endpoint requires a host".to_string(),
            )
        })?
        .to_ascii_lowercase();
    let port = url.port_or_known_default().ok_or_else(|| {
        TraceRemoteRequestFailure::endpoint_invalid(
            "trace remote endpoint requires a known port".to_string(),
        )
    })?;
    let resolved_addrs = resolve_trace_upload_claim_issuer_host(&host, port)
        .await
        .map_err(|error| {
            TraceRemoteRequestFailure::dns_rejected(format!(
                "trace remote endpoint host resolution rejected: {error}"
            ))
        })?;
    let timeout = trace_remote_request_timeout();
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(5)))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("ironclaw-trace-commons-client")
        .resolve_to_addrs(&host, &resolved_addrs)
        .build()
        .map_err(|error| {
            TraceRemoteRequestFailure::request_failed("trace remote HTTP client", error)
        })
}
