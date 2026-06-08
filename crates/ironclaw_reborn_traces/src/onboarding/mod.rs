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
    let pending = DeviceKeypair::load_or_generate_pending(dir, &invite.invite_hash())?;

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
    if let Err(e) = DeviceKeypair::discard_pending(dir, &invite.invite_hash()) {
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
        ) && let Err(discard_err) = DeviceKeypair::discard_pending(dir, &invite.invite_hash())
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
mod tests {
    use super::*;

    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    use axum::{Router, extract::State, routing::post};

    struct MockIssuer {
        pub addr: SocketAddr,
        pub received: Arc<Mutex<Vec<serde_json::Value>>>,
    }

    /// Axum handler state for the mock issuer:
    /// (canned response JSON, status code, recorded request bodies).
    type MockState = Arc<(
        serde_json::Value,
        axum::http::StatusCode,
        Arc<Mutex<Vec<serde_json::Value>>>,
    )>;

    /// Sentinel `device_key_id` value: the mock replaces it with the id derived
    /// from the request's submitted public key (see handler).
    const ECHO_DEVICE_KEY_ID: &str = "ECHO_DEVICE_KEY_ID";

    /// Derive `sha256:<hex>` of the base64-standard-decoded public key bytes —
    /// mirrors `device_key::device_key_id_from_pubkey` (the server's scheme).
    fn derive_device_key_id(pubkey_b64: &str) -> Option<String> {
        use base64::Engine as _;
        use sha2::{Digest, Sha256};
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(pubkey_b64)
            .ok()?;
        Some(format!("sha256:{}", hex::encode(Sha256::digest(&bytes))))
    }

    /// Spawn a mock `/v1/onboard` server on 127.0.0.1:0.
    /// `make_response` receives the bound address so it can embed the correct
    /// `issuer_url` origin in the response JSON.
    async fn spawn_mock_issuer<F>(make_response: F, status: axum::http::StatusCode) -> MockIssuer
    where
        F: Fn(SocketAddr) -> serde_json::Value + Send + Sync + 'static,
    {
        let received: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));

        // Bind first so we know the addr before building the response JSON.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock issuer binds");
        let addr = listener.local_addr().expect("mock issuer local addr");

        // Build the response JSON eagerly now that we know the addr.
        let response_body = make_response(addr);

        // State shared with the handler: (response_json, status_code, received_log).
        let state: MockState = Arc::new((response_body, status, Arc::clone(&received)));

        async fn handler(
            State(state): State<MockState>,
            axum::Json(body): axum::Json<serde_json::Value>,
        ) -> axum::response::Response {
            let mut response = state.0.clone();
            // Realistic server behavior: derive device_key_id from the
            // contributor's submitted public key. If the canned response uses
            // the `ECHO_DEVICE_KEY_ID` sentinel, replace it with the value
            // derived from this request so the client's local cross-check
            // passes. Tests that want a *mismatch* set an explicit value.
            if response.get("device_key_id").and_then(|v| v.as_str()) == Some(ECHO_DEVICE_KEY_ID)
                && let Some(pubkey_b64) = body.get("device_public_key").and_then(|v| v.as_str())
                && let Some(derived) = derive_device_key_id(pubkey_b64)
            {
                response["device_key_id"] = serde_json::Value::String(derived);
            }
            state.2.lock().unwrap().push(body);
            let json_bytes = serde_json::to_vec(&response).unwrap();
            axum::response::Response::builder()
                .status(state.1)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(json_bytes))
                .unwrap()
        }

        let app = Router::new()
            .route("/v1/onboard", post(handler))
            .with_state(state);

        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        MockIssuer { addr, received }
    }

    fn ok_response(addr: SocketAddr, ingest_url: &str) -> serde_json::Value {
        serde_json::json!({
            "schema_version": "trace_commons.onboard_response.v1",
            "tenant_id": "tenant-a",
            "ingest_url": ingest_url,
            "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
            "audience": "trace-commons-ingest",
            "device_key_id": ECHO_DEVICE_KEY_ID,
        })
    }

    #[tokio::test]
    async fn successful_onboard_writes_policy_and_promotes_key() {
        let dir = tempfile::tempdir().unwrap();
        let mock = spawn_mock_issuer(
            |addr| ok_response(addr, "https://ingest.example.com"),
            axum::http::StatusCode::OK,
        )
        .await;
        let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST01", mock.addr.port());
        let consents = OnboardConsents {
            include_message_text: true,
            include_tool_payloads: false,
        };

        let outcome = onboard_at_dir(dir.path(), &invite_url, consents)
            .await
            .expect("onboard succeeds");

        // Outcome fields.
        assert_eq!(outcome.tenant_id, "tenant-a");
        assert_eq!(outcome.ingest_url, "https://ingest.example.com");
        assert_eq!(
            outcome.issuer_url,
            format!("http://127.0.0.1:{}", mock.addr.port())
        );

        // Policy file exists with correct fields.
        let policy_path = dir.path().join("policy.json");
        assert!(policy_path.exists(), "policy.json must be written");
        let policy: StandingTraceContributionPolicy =
            serde_json::from_str(&std::fs::read_to_string(&policy_path).unwrap()).unwrap();
        assert!(policy.enabled);
        assert_eq!(policy.auth_mode, TraceUploadAuthMode::DeviceKey);
        // upload_token_issuer_url must be the full claim endpoint, not just the origin.
        let expected_issuer_url = format!(
            "http://127.0.0.1:{}/v1/trace-upload-claim",
            mock.addr.port()
        );
        assert_eq!(
            policy.upload_token_issuer_url.as_deref(),
            Some(expected_issuer_url.as_str()),
            "upload_token_issuer_url must be the full claim endpoint (origin + /v1/trace-upload-claim)"
        );
        // Verify parsed path is /v1/trace-upload-claim and host matches invite.
        let issuer_parsed = reqwest::Url::parse(expected_issuer_url.as_str()).unwrap();
        assert_eq!(issuer_parsed.path(), "/v1/trace-upload-claim");
        assert_eq!(issuer_parsed.host_str().unwrap(), format!("127.0.0.1"));
        assert_eq!(
            policy.ingestion_endpoint.as_deref(),
            Some("https://ingest.example.com")
        );
        assert!(policy.include_message_text);
        assert!(!policy.include_tool_payloads);
        assert_eq!(
            policy.device_key_id.as_deref(),
            Some(outcome.device_key_id.as_str())
        );

        // Tenant key file exists; pending file gone.
        let invite = ParsedInvite::parse(&invite_url).unwrap();
        let tenant_key = dir.path().join(format!(
            "device_keys/{}.json",
            device_key::tenant_hash("tenant-a")
        ));
        assert!(tenant_key.exists(), "tenant key file must exist");
        let pending = dir
            .path()
            .join(format!("device_keys/pending/{}.json", invite.invite_hash()));
        assert!(
            !pending.exists(),
            "pending file must be gone after the post-policy-write finalize"
        );

        // Exactly 1 request received with a non-empty device_public_key.
        let received = mock.received.lock().unwrap();
        assert_eq!(received.len(), 1);
        let dkpub = received[0]["device_public_key"].as_str().unwrap();
        assert!(!dkpub.is_empty());
    }

    #[tokio::test]
    async fn issuer_url_mismatch_rejects_onboard() {
        let dir = tempfile::tempdir().unwrap();
        let mock = spawn_mock_issuer(
            |_addr| {
                serde_json::json!({
                    "schema_version": "trace_commons.onboard_response.v1",
                    "tenant_id": "tenant-a",
                    "ingest_url": "https://ingest.example.com",
                    "issuer_url": "https://evil.example.com",
                    "audience": "trace-commons-ingest",
                    "device_key_id": "sha256:x",
                })
            },
            axum::http::StatusCode::OK,
        )
        .await;
        let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST02", mock.addr.port());

        let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
            .await
            .expect_err("mismatch must be rejected");

        assert!(
            matches!(err, OnboardError::IssuerOriginMismatch { .. }),
            "expected IssuerOriginMismatch, got: {err}"
        );
        assert!(
            !dir.path().join("policy.json").exists(),
            "policy.json must not be written on mismatch"
        );
    }

    #[tokio::test]
    async fn invite_not_valid_discards_pending_key() {
        let dir = tempfile::tempdir().unwrap();
        let mock = spawn_mock_issuer(
            |_addr| serde_json::json!({ "error": "InviteNotValid" }),
            axum::http::StatusCode::FORBIDDEN,
        )
        .await;
        let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST03", mock.addr.port());

        let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
            .await
            .expect_err("terminal rejection must fail");

        assert!(
            matches!(
                err,
                OnboardError::InviteRejected(OnboardErrorCode::InviteNotValid)
            ),
            "expected InviteRejected(InviteNotValid), got: {err}"
        );

        // Pending key directory should be empty or not contain the pending file.
        let invite = ParsedInvite::parse(&invite_url).unwrap();
        let pending = dir
            .path()
            .join(format!("device_keys/pending/{}.json", invite.invite_hash()));
        assert!(
            !pending.exists(),
            "pending key must be discarded on InviteNotValid"
        );
    }

    #[tokio::test]
    async fn transient_server_error_keeps_pending_key() {
        let dir = tempfile::tempdir().unwrap();
        let mock = spawn_mock_issuer(
            |_addr| serde_json::json!("garbage body that is not a valid error"),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        )
        .await;
        let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST04", mock.addr.port());

        let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
            .await
            .expect_err("transient error must fail");

        assert!(
            matches!(err, OnboardError::Network { .. }),
            "expected Network error, got: {err}"
        );

        // Pending file must still exist for retry.
        let invite = ParsedInvite::parse(&invite_url).unwrap();
        let pending = dir
            .path()
            .join(format!("device_keys/pending/{}.json", invite.invite_hash()));
        assert!(
            pending.exists(),
            "pending key must be retained on transient error"
        );
    }

    #[tokio::test]
    async fn non_https_ingest_url_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mock = spawn_mock_issuer(
            |addr| {
                serde_json::json!({
                    "schema_version": "trace_commons.onboard_response.v1",
                    "tenant_id": "tenant-a",
                    "ingest_url": "http://ingest.example.com",
                    "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                    "audience": "trace-commons-ingest",
                    "device_key_id": ECHO_DEVICE_KEY_ID,
                })
            },
            axum::http::StatusCode::OK,
        )
        .await;
        let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST05", mock.addr.port());

        let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
            .await
            .expect_err("non-https ingest_url must be rejected");

        assert!(
            matches!(err, OnboardError::InsecureIngestUrl { .. }),
            "expected InsecureIngestUrl, got: {err}"
        );
    }

    /// Loopback http ingest_url is allowed (same rule as invite parsing).
    #[tokio::test]
    async fn loopback_ingest_url_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let mock = spawn_mock_issuer(
            |addr| {
                serde_json::json!({
                    "schema_version": "trace_commons.onboard_response.v1",
                    "tenant_id": "tenant-b",
                    "ingest_url": "http://127.0.0.1:9999",
                    "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                    "audience": "trace-commons-ingest",
                    "device_key_id": ECHO_DEVICE_KEY_ID,
                })
            },
            axum::http::StatusCode::OK,
        )
        .await;
        let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST05B", mock.addr.port());

        let outcome = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
            .await
            .expect("loopback ingest_url must be accepted");
        assert_eq!(outcome.ingest_url, "http://127.0.0.1:9999");
    }

    #[tokio::test]
    async fn community_urls_pass_through_when_https_and_drop_when_not() {
        let dir = tempfile::tempdir().unwrap();
        let mock = spawn_mock_issuer(
            |addr| {
                serde_json::json!({
                    "schema_version": "trace_commons.onboard_response.v1",
                    "tenant_id": "tenant-a",
                    "ingest_url": "https://ingest.example.com",
                    "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                    "audience": "trace-commons-ingest",
                    "device_key_id": ECHO_DEVICE_KEY_ID,
                    "profile_url": "https://tracecommons.ai/profile",
                    "leaderboard_url": "http://insecure.example.com/lb",
                    "community_url": "https://tracecommons.ai",
                })
            },
            axum::http::StatusCode::OK,
        )
        .await;
        let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST06", mock.addr.port());

        let outcome = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
            .await
            .expect("onboard succeeds");

        assert_eq!(
            outcome.profile_url.as_deref(),
            Some("https://tracecommons.ai/profile"),
            "HTTPS profile_url must pass through"
        );
        assert!(
            outcome.leaderboard_url.is_none(),
            "non-HTTPS leaderboard_url must be dropped"
        );
        assert_eq!(
            outcome.community_url.as_deref(),
            Some("https://tracecommons.ai"),
            "HTTPS community_url must pass through"
        );
    }

    #[tokio::test]
    async fn retry_after_transient_failure_reuses_same_keypair() {
        let dir = tempfile::tempdir().unwrap();

        // First call: 500 → Network error.
        let mock_fail = spawn_mock_issuer(
            |_addr| serde_json::json!("error"),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        )
        .await;
        let invite_url = format!(
            "http://127.0.0.1:{}/onboard#INVTEST07",
            mock_fail.addr.port()
        );
        let _ = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
            .await
            .expect_err("first call must fail");

        // Read the staged pending key's public key for later comparison.
        let invite = ParsedInvite::parse(&invite_url).unwrap();
        let pending_path = dir
            .path()
            .join(format!("device_keys/pending/{}.json", invite.invite_hash()));
        assert!(
            pending_path.exists(),
            "pending file must exist after transient failure"
        );
        let pending_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&pending_path).unwrap()).unwrap();
        let first_pubkey = pending_json["public_key"].as_str().unwrap().to_owned();

        // Second call with a working mock that echoes the correct origin.
        // We must use a new mock with the SAME invite URL prefix but a different host,
        // or use the same invite_url but parse invite code and re-use a new mock.
        // Since the invite code is bound to the mock origin, we need the second mock to
        // have the same origin that the invite URL points to. We can't reuse the same
        // addr since tokio drops the listener. Instead, we construct a new invite URL
        // pointing at a fresh mock that echoes its own addr.
        let mock_ok = spawn_mock_issuer(
            |addr| ok_response(addr, "https://ingest.example.com"),
            axum::http::StatusCode::OK,
        )
        .await;
        let invite_url2 = format!("http://127.0.0.1:{}/onboard#INVTEST07", mock_ok.addr.port());

        let outcome = onboard_at_dir(dir.path(), &invite_url2, OnboardConsents::default())
            .await
            .expect("second call succeeds");

        // The second call's request device_public_key is the NEW pending key
        // (different invite hash due to different port/origin, but same code INVTEST07).
        // Actually with the same code but different origin the invite_hash is the same
        // (hash is of code only). So the same pending file is reused.
        let second_received = mock_ok.received.lock().unwrap();
        assert_eq!(second_received.len(), 1);
        let second_pubkey = second_received[0]["device_public_key"].as_str().unwrap();
        assert_eq!(
            first_pubkey, second_pubkey,
            "retry must reuse the staged keypair (same public key)"
        );
        assert!(!outcome.device_key_id.is_empty());
    }

    #[tokio::test]
    async fn policy_write_failure_keeps_pending_and_retry_reuses_key() {
        let dir = tempfile::tempdir().unwrap();

        // Force the policy write to fail by pre-creating <dir>/policy.json as a
        // DIRECTORY: `write_json_file` renames a temp file onto that path, which
        // fails deterministically when the destination is a non-empty dir.
        let policy_path = dir.path().join("policy.json");
        std::fs::create_dir(&policy_path).unwrap();
        std::fs::write(policy_path.join("blocker"), b"x").unwrap();

        let mock = spawn_mock_issuer(
            |addr| ok_response(addr, "https://ingest.example.com"),
            axum::http::StatusCode::OK,
        )
        .await;
        let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST08", mock.addr.port());

        let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
            .await
            .expect_err("policy write must fail");
        assert!(
            matches!(err, OnboardError::Persist { .. }),
            "expected Persist, got: {err}"
        );

        // The pending key MUST survive the failed policy write (no lockout).
        let invite = ParsedInvite::parse(&invite_url).unwrap();
        let pending_path = dir
            .path()
            .join(format!("device_keys/pending/{}.json", invite.invite_hash()));
        assert!(
            pending_path.exists(),
            "pending key must survive a failed policy write"
        );
        let staged: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&pending_path).unwrap()).unwrap();
        let staged_key_id = staged["device_key_id"].as_str().unwrap().to_owned();

        // Clear the blocker so the retry's policy write can succeed.
        std::fs::remove_dir_all(dir.path().join("policy.json")).unwrap();

        // Retry against an OK mock (same dir, same invite code) → success with
        // the SAME device_key_id (server idempotency + reused pending key).
        let mock_ok = spawn_mock_issuer(
            |addr| ok_response(addr, "https://ingest.example.com"),
            axum::http::StatusCode::OK,
        )
        .await;
        let invite_url2 = format!("http://127.0.0.1:{}/onboard#INVTEST08", mock_ok.addr.port());
        let outcome = onboard_at_dir(dir.path(), &invite_url2, OnboardConsents::default())
            .await
            .expect("retry succeeds after the blocker is cleared");

        assert_eq!(
            outcome.device_key_id, staged_key_id,
            "retry must reuse the same device key"
        );
        assert!(dir.path().join("policy.json").is_file());
        assert!(
            !pending_path.exists(),
            "pending key removed after the successful retry finalize"
        );
    }

    #[tokio::test]
    async fn wrong_schema_version_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mock = spawn_mock_issuer(
            |addr| {
                serde_json::json!({
                    "schema_version": "trace_commons.onboard_response.v2",
                    "tenant_id": "tenant-a",
                    "ingest_url": "https://ingest.example.com",
                    "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                    "audience": "trace-commons-ingest",
                    "device_key_id": ECHO_DEVICE_KEY_ID,
                })
            },
            axum::http::StatusCode::OK,
        )
        .await;
        let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST09", mock.addr.port());

        let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
            .await
            .expect_err("unexpected schema_version must be rejected");
        assert!(
            matches!(err, OnboardError::MalformedResponse { .. }),
            "expected MalformedResponse, got: {err}"
        );
        assert!(!dir.path().join("policy.json").exists());
    }

    /// The policy's upload_token_issuer_url must be the full claim endpoint
    /// (origin + /v1/trace-upload-claim), not the bare origin.  A bare-origin
    /// value would cause fetch_trace_upload_claim_from_issuer to POST to the
    /// root instead of the claim endpoint.
    #[tokio::test]
    async fn policy_upload_token_issuer_url_is_claim_endpoint_not_origin() {
        let dir = tempfile::tempdir().unwrap();
        let mock = spawn_mock_issuer(
            |addr| ok_response(addr, "https://ingest.example.com"),
            axum::http::StatusCode::OK,
        )
        .await;
        let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST11", mock.addr.port());

        onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
            .await
            .expect("onboard succeeds");

        let policy: StandingTraceContributionPolicy =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("policy.json")).unwrap())
                .unwrap();

        let issuer_url = policy
            .upload_token_issuer_url
            .as_deref()
            .expect("upload_token_issuer_url must be set");

        // Must NOT be the bare origin.
        assert_ne!(
            issuer_url,
            format!("http://127.0.0.1:{}", mock.addr.port()).as_str(),
            "upload_token_issuer_url must not be the bare origin"
        );

        // Must be the full claim endpoint.
        assert_eq!(
            issuer_url,
            format!(
                "http://127.0.0.1:{}/v1/trace-upload-claim",
                mock.addr.port()
            )
            .as_str(),
            "upload_token_issuer_url must be origin + /v1/trace-upload-claim"
        );

        // Parse and assert path and host separately.
        let parsed = reqwest::Url::parse(issuer_url).expect("issuer_url must be parseable");
        assert_eq!(
            parsed.path(),
            "/v1/trace-upload-claim",
            "parsed path must be /v1/trace-upload-claim"
        );
        assert_eq!(
            parsed.host_str().unwrap(),
            "127.0.0.1",
            "host must match invite host"
        );
        // Note: a full end-to-end claim-fetch-path test asserting the POST hits
        // /v1/trace-upload-claim would require a mock claim endpoint; deferred
        // because the policy URL path assertion above captures the same invariant
        // (fetch_trace_upload_claim_from_issuer POSTs to parsed.clone() as-is).
    }

    /// Verify that a terminal invite rejection returns InviteRejected (the
    /// primary error), not a DeviceKey error, even if discard_pending fails.
    /// This is the normal path — discard should succeed, and the primary error
    /// must be returned.
    #[tokio::test]
    async fn terminal_rejection_returns_invite_rejected_as_primary_error() {
        let dir = tempfile::tempdir().unwrap();
        let mock = spawn_mock_issuer(
            |_addr| serde_json::json!({ "error": "InviteMalformed" }),
            axum::http::StatusCode::FORBIDDEN,
        )
        .await;
        let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST12", mock.addr.port());

        let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
            .await
            .expect_err("terminal rejection must fail");

        // The error must be InviteRejected (the primary), not DeviceKey.
        assert!(
            matches!(
                err,
                OnboardError::InviteRejected(OnboardErrorCode::InviteMalformed)
            ),
            "expected InviteRejected(InviteMalformed), got: {err}"
        );
    }

    #[tokio::test]
    async fn mismatched_device_key_id_rejected() {
        let dir = tempfile::tempdir().unwrap();
        // Explicit (non-sentinel) device_key_id that cannot match the locally
        // derived value → the cross-check must reject it as a tamper signal.
        let mock = spawn_mock_issuer(
            |addr| {
                serde_json::json!({
                    "schema_version": "trace_commons.onboard_response.v1",
                    "tenant_id": "tenant-a",
                    "ingest_url": "https://ingest.example.com",
                    "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                    "audience": "trace-commons-ingest",
                    "device_key_id": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                })
            },
            axum::http::StatusCode::OK,
        )
        .await;
        let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST10", mock.addr.port());

        let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
            .await
            .expect_err("mismatched device_key_id must be rejected");
        assert!(
            matches!(err, OnboardError::MalformedResponse { .. }),
            "expected MalformedResponse, got: {err}"
        );
        assert!(!dir.path().join("policy.json").exists());
    }

    // ── Fake-sink tests (no network) ──────────────────────────────────────────

    /// A no-network sink that returns a canned status + body and records the
    /// posted URL — proves the seam works without reqwest/loopback.
    struct FakeSink {
        status: u16,
        body: Vec<u8>,
        posted_url: Arc<Mutex<Option<String>>>,
    }

    #[async_trait]
    impl OnboardingHttpSink for FakeSink {
        async fn post_onboard(
            &self,
            url: &str,
            _body: Vec<u8>,
        ) -> Result<OnboardHttpResponse, OnboardError> {
            *self.posted_url.lock().unwrap() = Some(url.to_string());
            Ok(OnboardHttpResponse {
                status: self.status,
                body: self.body.clone(),
            })
        }
    }

    /// Build the canned 200 onboard response body that the client will accept,
    /// echoing the device_key_id derived from the request's pending public key.
    fn canned_ok_body(dir: &Path, invite_url: &str) -> Vec<u8> {
        // Stage the pending key the same way onboard_at_dir will, so we can
        // derive the device_key_id the client will cross-check against.
        let invite = ParsedInvite::parse(invite_url).unwrap();
        let pending = DeviceKeypair::load_or_generate_pending(dir, &invite.invite_hash()).unwrap();
        let device_key_id = derive_device_key_id(&pending.public_key_b64).unwrap();
        serde_json::to_vec(&serde_json::json!({
            "schema_version": "trace_commons.onboard_response.v1",
            "tenant_id": "tenant-fake",
            "ingest_url": "https://ingest.example.com",
            "issuer_url": invite.origin,
            "audience": "trace-commons-ingest",
            "device_key_id": device_key_id,
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn fake_sink_200_writes_policy_through_seam() {
        let dir = tempfile::tempdir().unwrap();
        // Use a loopback-shaped invite URL so origin anchoring passes; no server
        // is bound — the fake sink never touches the network.
        let invite_url = "http://127.0.0.1:7/onboard#INVFAKE01";
        let body = canned_ok_body(dir.path(), invite_url);
        let posted_url = Arc::new(Mutex::new(None));
        let sink = FakeSink {
            status: 200,
            body,
            posted_url: Arc::clone(&posted_url),
        };

        let outcome =
            onboard_at_dir_with_sink(dir.path(), invite_url, OnboardConsents::default(), &sink)
                .await
                .expect("fake-sink onboard succeeds");

        assert_eq!(outcome.tenant_id, "tenant-fake");
        assert!(
            dir.path().join("policy.json").exists(),
            "policy.json must be written through the sink seam"
        );
        assert_eq!(
            posted_url.lock().unwrap().as_deref(),
            Some("http://127.0.0.1:7/v1/onboard"),
            "sink must be posted the onboard endpoint"
        );
    }

    #[tokio::test]
    async fn fake_sink_403_invite_not_valid_discards_pending() {
        let dir = tempfile::tempdir().unwrap();
        let invite_url = "http://127.0.0.1:7/onboard#INVFAKE02";
        let sink = FakeSink {
            status: 403,
            body: serde_json::to_vec(&serde_json::json!({ "error": "InviteNotValid" })).unwrap(),
            posted_url: Arc::new(Mutex::new(None)),
        };

        let err =
            onboard_at_dir_with_sink(dir.path(), invite_url, OnboardConsents::default(), &sink)
                .await
                .expect_err("4xx invite rejection must fail through the sink");

        assert!(
            matches!(
                err,
                OnboardError::InviteRejected(OnboardErrorCode::InviteNotValid)
            ),
            "expected InviteRejected(InviteNotValid), got: {err}"
        );
        let invite = ParsedInvite::parse(invite_url).unwrap();
        let pending = dir
            .path()
            .join(format!("device_keys/pending/{}.json", invite.invite_hash()));
        assert!(
            !pending.exists(),
            "pending key must be discarded on InviteNotValid through the sink"
        );
    }
}
