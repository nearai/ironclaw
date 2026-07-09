//! Trace Commons agent onboarding orchestration (spec §2.3).
//!
//! Entry points:
//! - [`onboard`] — public, resolves the scoped dir then calls
//!   [`onboard_at_dir_with_sink`] with the caller-supplied HTTP sink.
//! - [`onboard_at_dir`] — dir-parameterised core using the default reqwest sink;
//!   unit-testable with tempdirs.
//! - [`onboard_at_dir_with_sink`] — dir-parameterised core with an injectable
//!   [`OnboardingHttpSink`], so host callers can route the POST through their
//!   own network-egress policy.

pub mod device_key;
pub mod invite;
pub mod protocol;

use std::path::Path;

use async_trait::async_trait;
use thiserror::Error;

pub use device_key::{DeviceKeyError, DeviceKeypair};
pub use invite::{InviteParseError, ParsedInvite};
pub use protocol::{OnboardErrorCode, OnboardRequest, OnboardResponse};

use crate::contribution::{
    ConsentScope, StandingTraceContributionPolicy, TraceUploadAuthMode,
    trace_contribution_dir_for_scope,
};
use protocol::{
    ONBOARD_REQUEST_SCHEMA_VERSION, ONBOARD_RESPONSE_SCHEMA_VERSION, OnboardClientInfo,
};

/// Maximum response body size we accept from the onboarding endpoint (64 KB).
const MAX_RESPONSE_BODY: usize = 64 * 1024;
/// Default HTTP timeout for the onboarding POST.
const ONBOARD_TIMEOUT_SECS: u64 = 10;
/// Path of the upload-claim endpoint on the issuer. The policy stores the full
/// URL (origin + this path) so that `fetch_trace_upload_claim_from_issuer` can
/// POST to it directly without appending any path itself.
const UPLOAD_CLAIM_PATH: &str = "/v1/trace-upload-claim";

// ── Public types ────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Copy)]
pub struct OnboardConsents {
    pub include_message_text: bool,
    pub include_tool_payloads: bool,
}

#[derive(Debug, Clone)]
pub struct OnboardOutcome {
    pub tenant_id: String,
    pub ingest_url: String,
    /// Anchored issuer origin — from the invite URL, NOT the response value.
    pub issuer_url: String,
    pub device_key_id: String,
    pub contributor_label: Option<String>,
    /// Browser navigation hints from the server. Sanitized: each is `None`
    /// unless HTTPS — never fatal, never part of trust anchoring.
    pub community_url: Option<String>,
    pub profile_url: Option<String>,
    pub leaderboard_url: Option<String>,
}

#[derive(Debug, Error)]
pub enum OnboardError {
    #[error(transparent)]
    InvalidInvite(#[from] InviteParseError),
    #[error(transparent)]
    DeviceKey(#[from] DeviceKeyError),
    #[error("the onboarding server rejected the invite: {0:?}")]
    InviteRejected(OnboardErrorCode),
    #[error(
        "onboarding response issuer_url ({response}) does not match the invite origin ({invite}); refusing"
    )]
    IssuerOriginMismatch { invite: String, response: String },
    #[error("onboarding response ingest_url is not https: {url}")]
    InsecureIngestUrl { url: String },
    #[error("could not reach the onboarding server: {reason}")]
    Network { reason: String },
    #[error("onboarding response was malformed: {reason}")]
    MalformedResponse { reason: String },
    #[error("failed to persist onboarding state: {reason}")]
    Persist { reason: String },
}

// ── HTTP sink seam ───────────────────────────────────────────────────────────

/// Raw HTTP response from the onboarding POST: status code + bounded body.
///
/// Implementations return the status and body even for non-2xx responses — the
/// onboarding module parses 4xx bodies for the typed invite-rejection code.
#[derive(Debug, Clone)]
pub struct OnboardHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Injectable HTTP transport for the onboarding POST.
///
/// This trait references only `reborn_traces`/std types so the crate stays
/// independent of `ironclaw_host_api`. The default implementation
/// ([`DefaultOnboardingHttpSink`]) uses a direct `reqwest` client; host callers
/// supply an implementation that routes through the runtime network-egress
/// policy so the agent-invoked tool cannot reach private/internal destinations.
#[async_trait]
pub trait OnboardingHttpSink: Send + Sync {
    /// POST `body` (already-serialized JSON) to `url` with
    /// `Accept: application/json`, returning the raw status + bounded body.
    ///
    /// Implementations MUST enforce: no redirect following, a connect/total
    /// timeout (`ONBOARD_TIMEOUT_SECS`), and a `MAX_RESPONSE_BODY` (64 KiB)
    /// response-body cap (returning [`OnboardError::MalformedResponse`] on
    /// overflow, [`OnboardError::Network`] on transport/policy failure). They
    /// MUST return the status + body even for non-2xx responses.
    async fn post_onboard(
        &self,
        url: &str,
        body: Vec<u8>,
    ) -> Result<OnboardHttpResponse, OnboardError>;
}

/// Default direct-`reqwest` sink: no host network-egress policy.
///
/// Used by [`onboard_at_dir`] (CLI/test path). It allows loopback HTTP, which
/// the onboarding unit tests rely on.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultOnboardingHttpSink;

#[async_trait]
impl OnboardingHttpSink for DefaultOnboardingHttpSink {
    async fn post_onboard(
        &self,
        url: &str,
        body: Vec<u8>,
    ) -> Result<OnboardHttpResponse, OnboardError> {
        use reqwest::redirect::Policy;
        use std::time::Duration;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(ONBOARD_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(ONBOARD_TIMEOUT_SECS).min(Duration::from_secs(3)))
            .redirect(Policy::none())
            .user_agent("ironclaw-trace-commons-onboard/0.1")
            .build()
            .map_err(|e| OnboardError::Network {
                reason: format!("failed to build HTTP client: {e}"),
            })?;

        let resp = client
            .post(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .body(body)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .send()
            .await
            .map_err(|e| OnboardError::Network {
                reason: e.to_string(),
            })?;

        let status = resp.status().as_u16();
        // Read the body with the MAX_RESPONSE_BODY cap enforced DURING chunked
        // reading so a hostile server cannot force a large allocation by
        // streaming an oversized body.
        let body = read_bounded_response(resp).await?;
        Ok(OnboardHttpResponse { status, body })
    }
}

// ── Public entry points ─────────────────────────────────────────────────────

/// Public entry: resolves the scoped trace-contribution dir, then runs the
/// onboarding flow.
///
/// `write_trace_policy_for_scope` is a thin wrapper around
/// `write_json_file(&trace_policy_path(scope), policy, "trace policy")` where
/// `trace_policy_path(scope) == trace_contribution_dir_for_scope(Some(scope)).join("policy.json")`.
/// Since `onboard_at_dir` already writes to `<dir>/policy.json` via the same
/// `write_json_file` call, and `dir == trace_contribution_dir_for_scope(Some(scope))`,
/// the dir-parameterised write is equivalent and no extra step is needed here.
pub async fn onboard(
    scope: &str,
    invite_url: &str,
    consents: OnboardConsents,
    sink: &dyn OnboardingHttpSink,
) -> Result<OnboardOutcome, OnboardError> {
    let dir = trace_contribution_dir_for_scope(Some(scope));
    onboard_at_dir_with_sink(&dir, invite_url, consents, sink).await
}

/// Instance-wide enrollment: identical to [`onboard`] but writes the resulting
/// `StandingTraceContributionPolicy` to the instance-level location
/// (`trace_contribution_dir_for_scope(None)`), so all users without their own
/// personal-invite enrollment inherit it via `resolve_trace_credentials`.
///
/// This is an admin-only operation at the call boundary (the caller must gate
/// it: host-shell possession for the CLI, an admin identity for any future
/// product surface); the function itself only knows it targets the base dir.
pub async fn onboard_instance_with_sink(
    invite_url: &str,
    consents: OnboardConsents,
    sink: &dyn OnboardingHttpSink,
) -> Result<OnboardOutcome, OnboardError> {
    let dir = trace_contribution_dir_for_scope(None);
    onboard_at_dir_with_sink(&dir, invite_url, consents, sink).await
}

/// Base-dir-parameterised instance enrollment using the default
/// direct-`reqwest` sink — the admin CLI path (`ironclaw-reborn traces
/// enroll-instance`), where host-shell possession is the admin gate and there
/// is no host egress pipeline to route through. Targets
/// `<base>/trace_contributions/` (the scope-`None` location), so all users
/// without a personal enrollment inherit it via `resolve_trace_credentials`.
/// Tests supply an isolated tempdir base.
pub async fn onboard_instance_at_base(
    base_dir: &Path,
    invite_url: &str,
    consents: OnboardConsents,
) -> Result<OnboardOutcome, OnboardError> {
    let dir = crate::contribution::trace_contribution_dir_for_scope_at(base_dir, None);
    onboard_at_dir(&dir, invite_url, consents).await
}

/// Dir-parameterised core using the default direct-`reqwest` sink —
/// unit-testable with tempdirs (loopback mocks). Thin wrapper around
/// [`onboard_at_dir_with_sink`].
pub async fn onboard_at_dir(
    dir: &Path,
    invite_url: &str,
    consents: OnboardConsents,
) -> Result<OnboardOutcome, OnboardError> {
    onboard_at_dir_with_sink(dir, invite_url, consents, &DefaultOnboardingHttpSink).await
}

/// Dir-parameterised core with an injectable HTTP sink. The sink performs the
/// onboarding POST (transport + policy); serialization, status→error-code
/// parsing, terminal-key-discard, trust-anchoring, and policy write all stay
/// here.
pub async fn onboard_at_dir_with_sink(
    dir: &Path,
    invite_url: &str,
    consents: OnboardConsents,
    sink: &dyn OnboardingHttpSink,
) -> Result<OnboardOutcome, OnboardError> {
    // Step 1: Parse the invite URL (trust root).
    let invite = ParsedInvite::parse(invite_url)?;

    // Step 2: Stage the keypair BEFORE any network call (retry-safe).
    let pending = DeviceKeypair::load_or_generate_pending(dir, &invite.pending_key_hash())?;

    // Step 3: POST to the onboard endpoint. Terminal invite rejections discard
    // the pending key inside this call; transient failures keep it for retry.
    let response_bytes = post_onboard_request(dir, &invite, &pending, sink).await?;

    // Step 4: Parse + schema-validate the response body.
    let response = parse_onboard_response(response_bytes)?;

    // Step 5: Trust anchoring — issuer_url origin must equal invite origin.
    let response_issuer_origin =
        normalize_origin(&response.issuer_url).ok_or_else(|| OnboardError::MalformedResponse {
            reason: format!("issuer_url is not a valid URL: {}", response.issuer_url),
        })?;
    if response_issuer_origin != invite.origin {
        return Err(OnboardError::IssuerOriginMismatch {
            invite: invite.origin.clone(),
            response: response_issuer_origin,
        });
    }

    // Step 6: Cross-check the server's echoed device_key_id against the value
    // we derived locally from our own public key. We never trust the response
    // value (policy uses the local key); a disagreement is a tamper/identity
    // signal, so reject it as malformed.
    if response.device_key_id != pending.device_key_id {
        return Err(OnboardError::MalformedResponse {
            reason: "response device_key_id does not match the locally derived key".to_string(),
        });
    }

    // Step 7: ingest_url must be HTTPS (loopback http allowed for dev/tests).
    ensure_https_or_loopback_url(&response.ingest_url)?;

    // Step 8: Write the tenant key file (pending file deliberately kept).
    let key = pending.promote(dir, &response.tenant_id)?;

    // Step 9: Write the standing contribution policy. If this fails the pending
    // file still exists, so a retry reuses the same key (spec §2.2 — no
    // partial-failure lockout).
    write_policy_at_dir(dir, &key, &invite, &response, consents)?;

    // Step 10: Finalize — remove the pending file only now that both the tenant
    // key file and the policy are durably written. This is best-effort: the
    // onboarding has durably succeeded at this point, so a failure to delete
    // the pending file must NOT fail the call. A stale pending file is
    // harmless and self-heals — any later retry reloads the same key, performs
    // the idempotent tenant-file + policy overwrite, and discards it again.
    if let Err(e) = DeviceKeypair::discard_pending(dir, &invite.pending_key_hash()) {
        tracing::debug!("onboarding finalize: failed to remove pending device key (harmless): {e}");
    }

    // Step 11: Sanitize browser-nav hints — drop any non-HTTPS URL (never fatal).
    let sanitize_nav = |u: Option<String>| u.filter(|s| s.starts_with("https://"));

    // Return the outcome. issuer_url is the anchored invite-derived value.
    Ok(OnboardOutcome {
        tenant_id: response.tenant_id,
        ingest_url: response.ingest_url,
        issuer_url: invite.origin,
        device_key_id: key.device_key_id,
        contributor_label: response.contributor_label,
        community_url: sanitize_nav(response.community_url),
        profile_url: sanitize_nav(response.profile_url),
        leaderboard_url: sanitize_nav(response.leaderboard_url),
    })
}

// ── Private helpers ─────────────────────────────────────────────────────────

/// Serialize the onboard request, POST it through `sink`, and return the raw
/// response bytes. Handles terminal invite rejections (discard pending key) and
/// transient errors (keep pending key for retry). Transport/policy/body-cap
/// enforcement lives in the sink; status→error-code mapping stays here.
async fn post_onboard_request(
    dir: &Path,
    invite: &ParsedInvite,
    pending: &DeviceKeypair,
    sink: &dyn OnboardingHttpSink,
) -> Result<Vec<u8>, OnboardError> {
    let request_body = OnboardRequest {
        schema_version: ONBOARD_REQUEST_SCHEMA_VERSION,
        invite_code: invite.code.clone(),
        device_public_key: pending.public_key_b64.clone(),
        client_info: OnboardClientInfo {
            agent: "ironclaw".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    };
    let body = serde_json::to_vec(&request_body).map_err(|e| OnboardError::Network {
        reason: format!("failed to serialize onboard request: {e}"),
    })?;

    let response = sink.post_onboard(&invite.onboard_endpoint(), body).await?;
    let status = response.status;
    let body_bytes = response.body;

    if !(200..300).contains(&status) {
        // Try to parse a typed error code from the body.
        let error_code = serde_json::from_slice::<serde_json::Value>(&body_bytes)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_owned))
            .map(|code| OnboardErrorCode::parse(&code));

        let code = match (status, error_code) {
            (_, Some(code)) => code,
            (429, None) => OnboardErrorCode::OnboardRateLimited,
            _ => {
                // Transient — keep pending key for retry.
                return Err(OnboardError::Network {
                    reason: format!("onboarding server returned HTTP {status}"),
                });
            }
        };

        // Terminal invite rejections: discard pending key. If discard fails, log
        // and fall through — InviteRejected is the primary error the caller needs.
        if matches!(
            code,
            OnboardErrorCode::InviteNotValid
                | OnboardErrorCode::InviteMalformed
                | OnboardErrorCode::DeviceKeyMalformed
        ) && let Err(discard_err) =
            DeviceKeypair::discard_pending(dir, &invite.pending_key_hash())
        {
            // Log cleanup failure (no key material; path is local and low-sensitivity).
            tracing::debug!(
                error_kind = %discard_err,
                "failed to discard pending device key after terminal invite rejection; \
                 continuing to return the primary error"
            );
            // Fall through — the caller needs InviteRejected, not a DeviceKey error.
        }
        // RateLimited is transient. Unknown = optimistically transient (pending
        // key kept) — an unrecognized code may be a newer transient condition,
        // so we do not destroy the key and a retry remains possible.

        return Err(OnboardError::InviteRejected(code));
    }

    Ok(body_bytes)
}

/// Read a response body into a `Vec<u8>`, enforcing the `MAX_RESPONSE_BODY`
/// cap per-chunk during streaming so a hostile server cannot force a large
/// allocation. Mirrors `contribution::read_bounded_trace_upload_claim_response`.
async fn read_bounded_response(mut resp: reqwest::Response) -> Result<Vec<u8>, OnboardError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| OnboardError::Network {
        reason: format!("reading response body: {e}"),
    })? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_RESPONSE_BODY {
            return Err(OnboardError::MalformedResponse {
                reason: format!("response body exceeds {MAX_RESPONSE_BODY} bytes"),
            });
        }
    }
    Ok(bytes)
}

/// Parse the response body into `OnboardResponse`. On serde error returns
/// `MalformedResponse` (we do NOT discard the pending key — the server may
/// have successfully registered us, retrying is safe due to server idempotency).
fn parse_onboard_response(body: Vec<u8>) -> Result<OnboardResponse, OnboardError> {
    let response: OnboardResponse =
        serde_json::from_slice(&body).map_err(|e| OnboardError::MalformedResponse {
            reason: format!("failed to parse onboard response: {e}"),
        })?;
    // Defense-in-depth: reject an unexpected schema version outright rather
    // than silently coercing fields from an incompatible contract.
    if response.schema_version != ONBOARD_RESPONSE_SCHEMA_VERSION {
        return Err(OnboardError::MalformedResponse {
            reason: format!(
                "unexpected schema_version {:?} (expected {ONBOARD_RESPONSE_SCHEMA_VERSION:?})",
                response.schema_version
            ),
        });
    }
    Ok(response)
}

/// Return the scheme://host[:port] origin of a URL string, or `None` if the
/// URL is unparseable or has no host. Delegates to the shared helper in
/// `invite.rs` so origin-building logic is not duplicated.
fn normalize_origin(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| invite::origin_of(&u))
}

/// Reject URLs that are neither HTTPS nor loopback-HTTP. Same rule as
/// `ParsedInvite::parse`. Returns `InsecureIngestUrl` on failure.
fn ensure_https_or_loopback_url(url: &str) -> Result<(), OnboardError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| OnboardError::InsecureIngestUrl {
        url: url.to_string(),
    })?;
    // Reject embedded userinfo (`https://user:pass@host/...`) before persisting:
    // a server-controlled onboarding response could otherwise smuggle raw
    // credentials into policy.json and every later contribution/profile request
    // built from this endpoint.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(OnboardError::InsecureIngestUrl {
            url: url.to_string(),
        });
    }
    // `host_str()` keeps IPv6 brackets; `invite::host_only` strips them (one
    // source of truth for bracket handling, shared with invite parsing).
    let bare = invite::host_only(parsed.host_str().unwrap_or("")).to_ascii_lowercase();
    if invite::is_https_or_loopback(parsed.scheme(), &bare) {
        Ok(())
    } else {
        Err(OnboardError::InsecureIngestUrl {
            url: url.to_string(),
        })
    }
}

/// Build the `StandingTraceContributionPolicy` and atomically write it to
/// `<dir>/policy.json`.
fn write_policy_at_dir(
    dir: &Path,
    key: &DeviceKeypair,
    invite: &ParsedInvite,
    response: &OnboardResponse,
    consents: OnboardConsents,
) -> Result<(), OnboardError> {
    use std::collections::BTreeSet;

    let policy = StandingTraceContributionPolicy {
        enabled: true,
        auth_mode: TraceUploadAuthMode::DeviceKey,
        device_key_id: Some(key.device_key_id.clone()),
        ingestion_endpoint: Some(response.ingest_url.clone()),
        // upload_token_issuer_url must be the full claim endpoint that
        // fetch_trace_upload_claim_from_issuer POSTs to as-is. The issuer
        // serves upload claims at UPLOAD_CLAIM_PATH, same origin as /v1/onboard.
        // Trust-anchoring still uses invite.origin (origin-only compare in step 5);
        // the allowed-host check is path-independent.
        upload_token_issuer_url: Some(format!("{}{UPLOAD_CLAIM_PATH}", invite.origin)),
        upload_token_issuer_allowed_hosts: {
            let mut s = BTreeSet::new();
            s.insert(invite.issuer_host.clone());
            s
        },
        upload_token_audience: Some(response.audience.clone()),
        upload_token_tenant_id: Some(response.tenant_id.clone()),
        include_message_text: consents.include_message_text,
        include_tool_payloads: consents.include_tool_payloads,
        // default_scope is already ConsentScope::DebuggingEvaluation in Default.
        default_scope: ConsentScope::DebuggingEvaluation,
        ..StandingTraceContributionPolicy::default()
    };

    crate::contribution::write_json_file(&dir.join("policy.json"), &policy, "trace policy").map_err(
        |e| OnboardError::Persist {
            reason: e.to_string(),
        },
    )
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
