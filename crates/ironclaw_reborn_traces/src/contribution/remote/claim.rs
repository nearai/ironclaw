//! Upload-claim minting: the claim context and credential providers, the
//! issuer request/response pair and its bounded reads, the pinned HTTP sink,
//! the remote-failure taxonomy, and the issuer SSRF guards.

use crate::redaction::redact_sensitive_json;
use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;
use uuid::Uuid;

use crate::contribution::*;

#[derive(Debug, Clone)]
pub(crate) struct TraceUploadClaimContext {
    pub(crate) trace_id: Option<Uuid>,
    pub(crate) submission_id: Option<Uuid>,
    pub(crate) consent_scopes: Vec<ConsentScope>,
    pub(crate) allowed_uses: Vec<TraceAllowedUse>,
    /// Base directory of the user scope (e.g. `trace_contribution_dir_for_scope(scope)`).
    /// Required for `TraceUploadAuthMode::DeviceKey` — the device key is loaded from
    /// this directory.  `None` for callers that do not have a scope context (legacy
    /// CLI paths, static-token paths) which is fine as long as `auth_mode` is
    /// `WorkloadTokenEnv`.
    pub(crate) scope_dir: Option<PathBuf>,
    /// Per-user pseudonymous subject (from `resolve_trace_credentials`). When
    /// set and auth_mode is DeviceKey, it is sent to the issuer so the minted
    /// claim's principal is per-user under the shared instance device key.
    /// `None` for the personal-invite model (device key already 1:1 with user).
    pub(crate) subject: Option<String>,
}

impl TraceUploadClaimContext {
    pub(crate) fn for_envelope(envelope: &TraceContributionEnvelope) -> Self {
        Self {
            trace_id: Some(envelope.trace_id),
            submission_id: Some(envelope.submission_id),
            consent_scopes: envelope.consent.scopes.clone(),
            allowed_uses: envelope.trace_card.allowed_uses.clone(),
            scope_dir: None,
            subject: None,
        }
    }

    pub(crate) fn for_status_sync() -> Self {
        Self {
            trace_id: None,
            submission_id: None,
            consent_scopes: Vec::new(),
            allowed_uses: Vec::new(),
            scope_dir: None,
            subject: None,
        }
    }

    pub(crate) fn for_submission_id(submission_id: Uuid) -> Self {
        Self {
            trace_id: None,
            submission_id: Some(submission_id),
            consent_scopes: Vec::new(),
            allowed_uses: Vec::new(),
            scope_dir: None,
            subject: None,
        }
    }

    /// Attach the scope's base directory so that `DeviceKey` auth mode can
    /// locate the per-tenant keypair.
    pub(crate) fn with_scope_dir(mut self, dir: PathBuf) -> Self {
        self.scope_dir = Some(dir);
        self
    }

    /// Attach the per-user pseudonymous subject from `resolve_trace_credentials`.
    /// For instance-enrolled users this is `local_pseudonymous_contributor_id(scope)`;
    /// for personal-invite enrollment and paths with no user context it is `None`.
    pub(crate) fn with_subject(mut self, subject: Option<String>) -> Self {
        self.subject = subject;
        self
    }

    /// Context for account-management calls (e.g. minting a one-time login
    /// link). No trace or submission identity, no consent scopes — the caller
    /// is not submitting a trace.  Callers should chain `.with_scope_dir()` to
    /// supply the tenant keypair directory when `DeviceKey` auth is active.
    pub(crate) fn for_account(subject: Option<String>) -> Self {
        Self {
            trace_id: None,
            submission_id: None,
            consent_scopes: Vec::new(),
            allowed_uses: Vec::new(),
            scope_dir: None,
            subject,
        }
    }
}

#[async_trait]
pub(crate) trait TraceUploadCredentialProvider: Send + Sync {
    async fn bearer_token(
        &self,
        policy: &StandingTraceContributionPolicy,
        context: &TraceUploadClaimContext,
        force_refresh: bool,
    ) -> anyhow::Result<String>;
}

pub(crate) struct DefaultTraceUploadCredentialProvider;

pub(crate) struct StaticEnvTraceUploadCredentialProvider<'a> {
    pub(crate) bearer_token_env: &'a str,
}

#[derive(Debug, Clone)]
pub(crate) struct CachedTraceUploadClaim {
    token: String,
    refresh_after: DateTime<Utc>,
}

pub(crate) static TRACE_UPLOAD_CLAIM_CACHE: LazyLock<
    std::sync::Mutex<BTreeMap<String, CachedTraceUploadClaim>>,
> = LazyLock::new(|| std::sync::Mutex::new(BTreeMap::new()));

#[derive(Debug, Serialize)]
pub(crate) struct TraceUploadClaimIssuerRequest {
    schema_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tenant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audience: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    submission_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    consent_scopes: Vec<ConsentScope>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_uses: Vec<TraceAllowedUse>,
    requested_at: DateTime<Utc>,
    /// Pilot invite code mirrored from the standing policy. Server-side
    /// the canonical source is `WorkloadClaims.invite_code` (signed); this
    /// field is sent in the body as forward-compat for a future server
    /// slice that may accept it from either source. Omitted when the
    /// policy has no `upload_token_invite_code` set.
    #[serde(skip_serializing_if = "Option::is_none")]
    invite_code: Option<String>,
    /// Per-user subject; only sent in DeviceKey mode. The server (Slice 0)
    /// derives a per-user principal from it. Omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TraceUploadClaimIssuerResponse {
    pub(crate) access_token: String,
    #[serde(default)]
    pub(crate) token_type: Option<String>,
    #[serde(default)]
    pub(crate) expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(crate) expires_in: Option<i64>,
}

#[derive(Debug)]
pub(crate) struct TraceRemoteRequestFailure {
    status: Option<reqwest::StatusCode>,
    pub(crate) kind: TraceQueueTelemetryFailureKind,
    message: String,
    source: Option<reqwest::Error>,
}

impl TraceRemoteRequestFailure {
    pub(crate) fn request_failed(operation: &'static str, error: reqwest::Error) -> Self {
        let status = error.status();
        let kind = trace_remote_request_failure_kind_for_reqwest_error(&error);
        Self {
            status,
            kind,
            message: format!("{operation} request failed: {error}"),
            source: Some(error),
        }
    }

    pub(crate) fn http_rejection(
        operation: &'static str,
        status: reqwest::StatusCode,
        body: String,
    ) -> Self {
        let safe_body = safe_trace_remote_rejection_body(&body);
        let message = if safe_body.is_empty() {
            format!("{operation} rejected by {status}")
        } else {
            format!("{operation} rejected by {status}: {safe_body}")
        };
        let kind = if matches!(
            status,
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            TraceQueueTelemetryFailureKind::Credential
        } else {
            TraceQueueTelemetryFailureKind::HttpRejection
        };
        Self {
            status: Some(status),
            kind,
            message,
            source: None,
        }
    }

    pub(crate) fn auth_rejection(&self) -> bool {
        matches!(
            self.status,
            Some(reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN)
        )
    }

    pub(crate) fn endpoint_invalid(message: String) -> Self {
        Self {
            status: None,
            kind: TraceQueueTelemetryFailureKind::Endpoint,
            message,
            source: None,
        }
    }

    pub(crate) fn dns_rejected(message: String) -> Self {
        Self {
            status: None,
            kind: TraceQueueTelemetryFailureKind::NetworkDns,
            message,
            source: None,
        }
    }
}

impl std::fmt::Display for TraceRemoteRequestFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TraceRemoteRequestFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|error| error as &(dyn Error + 'static))
    }
}

pub(crate) const TRACE_REMOTE_REJECTION_BODY_MAX_CHARS: usize = 512;

pub(crate) fn safe_trace_remote_rejection_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let sanitized = serde_json::from_str::<Value>(trimmed)
        .map(|value| redact_sensitive_json(&value).to_string())
        .unwrap_or_else(|_| trimmed.split_whitespace().collect::<Vec<_>>().join(" "));
    let mut chars = sanitized.chars();
    let mut bounded = chars
        .by_ref()
        .take(TRACE_REMOTE_REJECTION_BODY_MAX_CHARS)
        .collect::<String>();
    if chars.next().is_some() {
        bounded.push_str("...");
    }
    bounded
}

pub(crate) fn trace_remote_request_failure_kind_for_reqwest_error(
    error: &reqwest::Error,
) -> TraceQueueTelemetryFailureKind {
    if let Some(kind) = trace_queue_telemetry_failure_kind_for_error_source(error) {
        return kind;
    }
    let message = error_chain_message(error).to_ascii_lowercase();
    if error.is_timeout()
        || message.contains("timed out")
        || message.contains("timeout")
        || message.contains("deadline elapsed")
    {
        TraceQueueTelemetryFailureKind::NetworkTimeout
    } else if message.contains("network is unreachable")
        || message.contains("no route to host")
        || message.contains("offline")
        || message.contains("internet connection appears to be offline")
    {
        TraceQueueTelemetryFailureKind::NetworkOffline
    } else if message.contains("dns")
        || message.contains("failed to lookup")
        || message.contains("failed to resolve")
        || message.contains("name or service not known")
        || message.contains("nodename nor servname")
    {
        TraceQueueTelemetryFailureKind::NetworkDns
    } else if message.contains("connection refused") || message.contains("refused") {
        TraceQueueTelemetryFailureKind::NetworkConnectionRefused
    } else {
        TraceQueueTelemetryFailureKind::Network
    }
}

pub(crate) fn trace_queue_telemetry_failure_kind_for_error_source(
    error: &(dyn Error + 'static),
) -> Option<TraceQueueTelemetryFailureKind> {
    let mut source = Some(error);
    while let Some(cause) = source {
        if let Some(reqwest_error) = cause.downcast_ref::<reqwest::Error>()
            && reqwest_error.is_timeout()
        {
            return Some(TraceQueueTelemetryFailureKind::NetworkTimeout);
        }
        if let Some(io_error) = cause.downcast_ref::<std::io::Error>()
            && let Some(kind) =
                trace_queue_telemetry_failure_kind_for_io_error_kind(io_error.kind())
        {
            return Some(kind);
        }
        source = cause.source();
    }
    None
}

pub(crate) fn trace_queue_telemetry_failure_kind_for_io_error_kind(
    kind: std::io::ErrorKind,
) -> Option<TraceQueueTelemetryFailureKind> {
    match kind {
        std::io::ErrorKind::TimedOut => Some(TraceQueueTelemetryFailureKind::NetworkTimeout),
        std::io::ErrorKind::ConnectionRefused => {
            Some(TraceQueueTelemetryFailureKind::NetworkConnectionRefused)
        }
        std::io::ErrorKind::AddrNotAvailable
        | std::io::ErrorKind::HostUnreachable
        | std::io::ErrorKind::NetworkDown
        | std::io::ErrorKind::NetworkUnreachable
        | std::io::ErrorKind::NotConnected => Some(TraceQueueTelemetryFailureKind::NetworkOffline),
        std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::ConnectionReset => {
            Some(TraceQueueTelemetryFailureKind::Network)
        }
        _ => None,
    }
}

pub(crate) fn error_chain_message(error: &(dyn Error + 'static)) -> String {
    let mut messages = vec![error.to_string()];
    let mut source = error.source();
    while let Some(error) = source {
        messages.push(error.to_string());
        source = error.source();
    }
    messages.join("\n")
}

#[async_trait]
impl TraceUploadCredentialProvider for StaticEnvTraceUploadCredentialProvider<'_> {
    async fn bearer_token(
        &self,
        _policy: &StandingTraceContributionPolicy,
        _context: &TraceUploadClaimContext,
        _force_refresh: bool,
    ) -> anyhow::Result<String> {
        trace_upload_static_env_bearer_token(self.bearer_token_env)
    }
}

#[async_trait]
impl TraceUploadCredentialProvider for DefaultTraceUploadCredentialProvider {
    async fn bearer_token(
        &self,
        policy: &StandingTraceContributionPolicy,
        context: &TraceUploadClaimContext,
        force_refresh: bool,
    ) -> anyhow::Result<String> {
        trace_upload_bearer_token_via(policy, context, force_refresh, None).await
    }
}

/// Sink-aware bearer mint. `sink == Some`: AGENT path — the upload-claim
/// issuer request routes through host `RuntimeHttpEgress` like every other
/// agent-driven network effect. `sink == None`: WORKER/CLI path — the direct
/// hardened reqwest client (unchanged [`DefaultTraceUploadCredentialProvider`]
/// behavior). Sink-based entry points (login-link, account-traces) MUST pass
/// their sink here rather than minting through the default provider, or the
/// claim request silently bypasses the deployment's egress policy.
pub(crate) async fn trace_upload_bearer_token_via(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
    force_refresh: bool,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<String> {
    if policy
        .upload_token_issuer_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty())
    {
        return trace_upload_issuer_claim_bearer_token(policy, context, force_refresh, sink).await;
    }
    trace_upload_static_env_bearer_token(&policy.bearer_token_env)
}

pub(crate) fn trace_upload_static_env_bearer_token(
    bearer_token_env: &str,
) -> anyhow::Result<String> {
    std::env::var(bearer_token_env).map_err(|_| {
        anyhow::anyhow!(
            "{} is not set; refusing to call Trace Commons without explicit API credentials",
            bearer_token_env
        )
    })
}

pub(crate) async fn trace_upload_issuer_claim_bearer_token(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
    force_refresh: bool,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<String> {
    let cache_key = trace_upload_claim_cache_key(policy, context)?;
    if !force_refresh && let Some(cached) = trace_upload_cached_claim(&cache_key, Utc::now()) {
        return Ok(cached);
    }

    let claim = fetch_trace_upload_claim_from_issuer(policy, context, sink).await?;
    if let Some(refresh_after) = trace_upload_claim_refresh_after(&claim, Utc::now()) {
        let mut cache = match TRACE_UPLOAD_CLAIM_CACHE.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        cache.insert(
            cache_key,
            CachedTraceUploadClaim {
                token: claim.access_token.clone(),
                refresh_after,
            },
        );
    }
    Ok(claim.access_token)
}

pub(crate) fn trace_upload_cached_claim(cache_key: &str, now: DateTime<Utc>) -> Option<String> {
    let cache = match TRACE_UPLOAD_CLAIM_CACHE.lock() {
        Ok(cache) => cache,
        Err(poisoned) => poisoned.into_inner(),
    };
    cache
        .get(cache_key)
        .filter(|cached| cached.refresh_after > now)
        .map(|cached| cached.token.clone())
}

pub(crate) fn trace_upload_claim_cache_key(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
) -> anyhow::Result<String> {
    let issuer = policy
        .upload_token_issuer_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Trace Commons upload token issuer URL is not configured"))?
        .trim();
    // Include invite_code in the cache key so rotating it forces a fresh
    // claim fetch (the issuer's mint binds a `policy_label` claim derived
    // from the active allowlist policy; serving a cached token after the
    // operator changed the user's invite_code would mis-attribute traces).
    // Hash the invite code to keep the operator-secret out of the in-memory
    // cache key; distinct raw codes still produce distinct hashes, so cache
    // separation is preserved.
    let invite_code_key = policy
        .upload_token_invite_code
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|code| format!("sha256:{}", hex::encode(Sha256::digest(code.as_bytes()))))
        .unwrap_or_default();
    // In DeviceKey mode, different scopes within the same tenant would otherwise
    // collide on the same cache key (they share the same issuer/tenant/audience).
    // Include a hash of the scope_dir path to ensure each scope gets its own
    // cached claim.  WorkloadTokenEnv mode has no scope concept so scope_dir is
    // always None there — no change in that path.
    let scope_dir_key = match policy.auth_mode {
        TraceUploadAuthMode::DeviceKey => context
            .scope_dir
            .as_ref()
            .map(|p| {
                format!(
                    "sha256:{}",
                    hex::encode(Sha256::digest(p.to_string_lossy().as_bytes()))
                )
            })
            .unwrap_or_default(),
        TraceUploadAuthMode::WorkloadTokenEnv => String::new(),
    };
    // Under instance enrollment every user shares the SAME instance device-key
    // dir (scope `None`), so `scope_dir_key` is identical across users — the
    // per-user `subject` is what distinguishes their minted claims. Omitting it
    // would let a claim minted for one subject be served from cache to another,
    // mis-attributing traces / leaking across users.
    //
    // The key MUST hash the EXACT bytes the issuer request sends (see
    // `build_trace_upload_claim_issuer_request`: DeviceKey carries `subject`,
    // WorkloadTokenEnv never does) with a `None`/`Some` discriminator. Trimming
    // or collapsing empties here (the old behavior) let `None`, `Some("")`, and
    // whitespace variants share a key while minting different payloads.
    let payload_subject = match policy.auth_mode {
        TraceUploadAuthMode::DeviceKey => context.subject.as_deref(),
        TraceUploadAuthMode::WorkloadTokenEnv => None,
    };
    let subject_key = match payload_subject {
        Some(subject) => format!(
            "some:sha256:{}",
            hex::encode(Sha256::digest(subject.as_bytes()))
        ),
        None => "none".to_string(),
    };
    Ok(format!(
        "{}|tenant={}|audience={}|scopes={}|uses={}|workload_env={}|invite_code={}|scope_dir={}|subject={}",
        issuer,
        policy.upload_token_tenant_id.as_deref().unwrap_or_default(),
        policy.upload_token_audience.as_deref().unwrap_or_default(),
        trace_upload_claim_scope_key(&context.consent_scopes),
        trace_upload_claim_use_key(&context.allowed_uses),
        policy
            .upload_token_workload_token_env
            .as_deref()
            .unwrap_or_default(),
        invite_code_key,
        scope_dir_key,
        subject_key,
    ))
}

pub(crate) fn trace_upload_claim_scope_key(scopes: &[ConsentScope]) -> String {
    scopes
        .iter()
        .map(|scope| format!("{scope:?}"))
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn trace_upload_claim_use_key(uses: &[TraceAllowedUse]) -> String {
    uses.iter()
        .map(|allowed_use| format!("{allowed_use:?}"))
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn trace_upload_claim_refresh_after(
    response: &TraceUploadClaimIssuerResponse,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let expires_at = match response.expires_at {
        Some(expires_at) => expires_at,
        None => {
            let seconds = response.expires_in?;
            if seconds <= 0 {
                return None;
            }
            now.checked_add_signed(chrono::Duration::seconds(seconds))?
        }
    };
    let refresh_after =
        expires_at - chrono::Duration::seconds(TRACE_UPLOAD_CLAIM_REFRESH_SKEW_SECONDS);
    (refresh_after > now).then_some(refresh_after)
}

/// Typed PilotAllowlist refusal labels returned by the upload-claim issuer
/// when its `pilot_allowlist` gate refuses to mint a claim. Parsing into an
/// enum keeps the diagnostic mapping closed: any unknown label falls through
/// to the generic HTTP-status diagnostic, which is what we want for
/// future-extension safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PilotAllowlistRefusal {
    NotMatched,
    InviteCodeMissing,
    Stale,
    Malformed,
}

impl PilotAllowlistRefusal {
    fn from_label(label: &str) -> Option<Self> {
        match label {
            "PilotAllowlistNotMatched" => Some(Self::NotMatched),
            "PilotAllowlistInviteCodeMissing" => Some(Self::InviteCodeMissing),
            "PilotAllowlistStale" => Some(Self::Stale),
            "PilotAllowlistMalformed" => Some(Self::Malformed),
            _ => None,
        }
    }

    fn label_str(&self) -> &'static str {
        match self {
            Self::NotMatched => "PilotAllowlistNotMatched",
            Self::InviteCodeMissing => "PilotAllowlistInviteCodeMissing",
            Self::Stale => "PilotAllowlistStale",
            Self::Malformed => "PilotAllowlistMalformed",
        }
    }

    fn diagnostic(&self) -> &'static str {
        match self {
            Self::InviteCodeMissing => {
                "the workload token did not carry an invite_code claim. \
                 Re-run `ironclaw traces opt-in --upload-token-invite-code <CODE> ...` with the operator-issued code, \
                 or have your operator reissue a workload token that includes it."
            }
            Self::NotMatched => {
                "the invite code hash was not in the issuer's active allowlist. \
                 Confirm the code with your operator; it may have been rotated or revoked."
            }
            Self::Stale => {
                "the issuer's allowlist snapshot is stale and the source has not reloaded successfully. \
                 This is transient on the issuer side — retry after the operator confirms recovery."
            }
            Self::Malformed => {
                "the issuer's allowlist source is failing to parse. \
                 This is an operator-side problem — escalate to the issuer admin."
            }
        }
    }
}

/// Build the `anyhow` error returned when the issuer rejects an upload-claim
/// request with a non-success HTTP status. Factored out so the
/// label-dispatch logic (typed PilotAllowlist diagnostics vs. generic HTTP
/// fallback) is unit-testable without spinning up a full HTTPS issuer.
pub(crate) fn build_trace_upload_claim_http_error(
    issuer_label: &str,
    status: u16,
    body_text: &str,
) -> anyhow::Error {
    let label = parse_trace_upload_claim_error_label(body_text);
    let refusal = label.as_deref().and_then(PilotAllowlistRefusal::from_label);
    if let Some(refusal) = refusal {
        return anyhow::anyhow!(
            "Trace Commons upload claim refused by {} ({}): {} — {}",
            issuer_label,
            status,
            refusal.label_str(),
            refusal.diagnostic(),
        );
    }
    anyhow::anyhow!(
        "failed to fetch Trace Commons upload claim from {}: HTTP {}{}",
        issuer_label,
        status,
        label
            .as_deref()
            .map(|l| format!(" ({l})"))
            .unwrap_or_default(),
    )
}

/// Returns the bearer credential to present to the upload-claim issuer.
///
/// - `TraceUploadAuthMode::DeviceKey`: self-signs a short-lived workload JWT
///   with the standaloneice keypair for the tenant.  The context must carry a
///   `scope_dir`.
/// - `TraceUploadAuthMode::WorkloadTokenEnv`: reads the workload token from
///   the environment variable named in the policy (existing behavior, byte-for-byte
///   identical to the inline block this replaces).
///
/// Returns `Ok(None)` when the policy has no `upload_token_workload_token_env`
/// configured and `auth_mode` is `WorkloadTokenEnv`, which means the caller
/// should proceed without a bearer credential (unauthenticated issuer).
pub(crate) async fn issuer_request_bearer(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
) -> anyhow::Result<Option<String>> {
    match policy.auth_mode {
        TraceUploadAuthMode::DeviceKey => {
            let tenant = policy.upload_token_tenant_id.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "device-key auth requires upload_token_tenant_id in the trace policy"
                )
            })?;
            let audience = policy.upload_token_audience.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "device-key auth requires upload_token_audience in the trace policy"
                )
            })?;
            let scope_dir = context.scope_dir.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "device-key auth requires a scope directory on the claim context; \
                     ensure the caller threads the user scope"
                )
            })?;
            let key = crate::onboarding::DeviceKeypair::load_for_tenant(scope_dir, tenant)
                .map_err(|e| anyhow::anyhow!("failed to load device key: {e}"))?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "trace policy is in device-key auth mode but no device key exists \
                         for tenant {tenant}; re-run onboarding"
                    )
                })?;
            Ok(Some(key.sign_workload_jwt(audience).map_err(|e| {
                anyhow::anyhow!("failed to sign workload JWT: {e}")
            })?))
        }
        TraceUploadAuthMode::WorkloadTokenEnv => {
            let Some(env_name) = policy.upload_token_workload_token_env.as_deref() else {
                return Ok(None);
            };
            if env_name.trim().is_empty() {
                return Ok(None);
            }
            let workload_token = std::env::var(env_name).map_err(|_| {
                anyhow::anyhow!(
                    "{} is not set; refusing to fetch Trace Commons upload claim without \
                     workload credentials",
                    env_name
                )
            })?;
            Ok(Some(workload_token))
        }
    }
}

/// Build the JSON body sent to the upload-claim issuer for a claim context.
/// Factored out of `fetch_trace_upload_claim_from_issuer` so the wire shape
/// (skip-serialized empty/None fields) is unit-testable without an HTTPS
/// issuer.
pub(crate) fn build_trace_upload_claim_issuer_request(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
) -> TraceUploadClaimIssuerRequest {
    // In DeviceKey mode the registered device key is the post-invite credential —
    // the server does not expect (and must not receive) an invite_code in the body.
    let invite_code = match policy.auth_mode {
        TraceUploadAuthMode::WorkloadTokenEnv => policy
            .upload_token_invite_code
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        TraceUploadAuthMode::DeviceKey => None,
    };
    // Per-user subject only applies to the device-key (instance) path; in
    // WorkloadTokenEnv mode the workload token already identifies the principal.
    let subject = match policy.auth_mode {
        TraceUploadAuthMode::DeviceKey => context.subject.clone(),
        TraceUploadAuthMode::WorkloadTokenEnv => None,
    };
    TraceUploadClaimIssuerRequest {
        schema_version: "ironclaw.trace_upload_claim_request.v1",
        tenant_id: policy.upload_token_tenant_id.clone(),
        audience: policy.upload_token_audience.clone(),
        trace_id: context.trace_id,
        submission_id: context.submission_id,
        consent_scopes: context.consent_scopes.clone(),
        allowed_uses: context.allowed_uses.clone(),
        requested_at: Utc::now(),
        invite_code,
        subject,
    }
}

/// Host-injected HTTP transport for AGENT-INVOKED Trace Commons contribution
/// writes (upload-claim mint, community-profile PUT/DELETE). When `Some`, these
/// run through the host `RuntimeHttpEgress` pipeline (private-IP filtering,
/// redaction, byte accounting). The background flush/sync worker and the CLI
/// pass `None` and keep their crate-local client (see `trace_remote_http_client`,
/// whose comment justifies why the worker lane intentionally bypasses egress).
#[async_trait]
pub trait ContributionHttpSink: Send + Sync {
    async fn execute(
        &self,
        request: ContributionHttpRequest,
    ) -> Result<ContributionHttpResponse, ContributionHttpError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContributionHttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

pub struct ContributionHttpRequest {
    pub method: ContributionHttpMethod,
    pub url: String,
    pub bearer_token: Option<String>,
    pub json_body: Option<Vec<u8>>,
    pub response_body_limit: u64,
    pub timeout_ms: u32,
}

#[derive(Debug)]
pub struct ContributionHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub struct ContributionHttpError {
    message: String,
}

impl ContributionHttpError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ContributionHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ContributionHttpError {}

/// Direct-transport [`ContributionHttpSink`] for trusted non-agent surfaces
/// (WebUI services, CLI). Applies the same hardening as the other direct
/// clients in this module: per-request pinned DNS resolution with
/// private/internal-IP rejection (`resolve_trace_upload_claim_issuer_host`),
/// no redirects, the request's own timeout, and a body read bounded DURING
/// streaming by the request's `response_body_limit`. Agent-path callers must
/// keep using the host-egress sink instead.
/// INVARIANT: request URLs handed to this sink must be derived from the
/// enrolled policy's trust-anchored endpoints (`account_login_links_url`,
/// `account_traces_url`, …) — never from caller/request input. The sink
/// attaches the caller's bearer to whatever URL it is given; keeping it
/// crate-private confines that to the vetted derivations in this module.
pub(crate) struct DirectPinnedContributionSink;

#[async_trait]
impl ContributionHttpSink for DirectPinnedContributionSink {
    async fn execute(
        &self,
        request: ContributionHttpRequest,
    ) -> Result<ContributionHttpResponse, ContributionHttpError> {
        let url = reqwest::Url::parse(&request.url)
            .map_err(|e| ContributionHttpError::new(format!("invalid request URL: {e}")))?;
        let host = url
            .host_str()
            .ok_or_else(|| ContributionHttpError::new("request URL requires a host"))?
            .to_ascii_lowercase();
        let port = url
            .port_or_known_default()
            .ok_or_else(|| ContributionHttpError::new("request URL requires a known port"))?;
        let resolved_addrs = resolve_trace_upload_claim_issuer_host(&host, port)
            .await
            .map_err(|e| ContributionHttpError::new(format!("host resolution rejected: {e}")))?;
        let timeout = Duration::from_millis(u64::from(request.timeout_ms));
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(3)))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("ironclaw-trace-commons-client")
            .resolve_to_addrs(&host, &resolved_addrs)
            .build()
            .map_err(|e| ContributionHttpError::new(format!("failed to build client: {e}")))?;

        let method = match request.method {
            ContributionHttpMethod::Get => reqwest::Method::GET,
            ContributionHttpMethod::Post => reqwest::Method::POST,
            ContributionHttpMethod::Put => reqwest::Method::PUT,
            ContributionHttpMethod::Delete => reqwest::Method::DELETE,
        };
        let mut builder = client
            .request(method, url)
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(token) = request.bearer_token {
            builder = builder.bearer_auth(token);
        }
        if let Some(body) = request.json_body {
            builder = builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        let mut response = builder
            .send()
            .await
            .map_err(|e| ContributionHttpError::new(format!("request failed: {e}")))?;
        let status = response.status().as_u16();
        // Enforce the cap DURING the chunked read so a hostile server cannot
        // force a large allocation by streaming an oversized body.
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| ContributionHttpError::new(format!("response read failed: {e}")))?
        {
            if body.len() as u64 + chunk.len() as u64 > request.response_body_limit {
                return Err(ContributionHttpError::new(format!(
                    "response body exceeds the {} byte limit",
                    request.response_body_limit
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(ContributionHttpResponse { status, body })
    }
}

/// Decode a host-egress response body into a bounded UTF-8 string, capping at
/// `TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES` (the host egress already enforced the
/// limit, but truncating defensively keeps a hostile body bounded). Lossy
/// decoding is acceptable — the body is parsed as JSON or scanned for an error
/// label, never echoed verbatim.
pub(crate) fn bounded_utf8_from_egress_body(mut body: Vec<u8>) -> String {
    body.truncate(TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES);
    String::from_utf8_lossy(&body).into_owned()
}

pub(crate) async fn fetch_trace_upload_claim_from_issuer(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<TraceUploadClaimIssuerResponse> {
    let issuer_url = policy.upload_token_issuer_url.as_deref().ok_or_else(|| {
        anyhow::anyhow!("Trace Commons upload token issuer URL is not configured")
    })?;
    let parsed =
        reqwest::Url::parse(issuer_url).context("invalid Trace Commons upload token issuer URL")?;
    validate_trace_upload_claim_issuer_url(&parsed, &policy.upload_token_issuer_allowed_hosts)?;
    let timeout = trace_upload_claim_issuer_timeout(policy)?;
    let request_body = build_trace_upload_claim_issuer_request(policy, context);
    let issuer_bearer = issuer_request_bearer(policy, context).await?;

    // Both branches converge on `(status, body_text)`, then share the status
    // check + JSON parse + response validation below.
    let (status, body_text): (u16, String) = if let Some(sink) = sink {
        // AGENT path: route through the host RuntimeHttpEgress pipeline. The
        // egress performs its own private-IP filtering and DNS resolution, so
        // this branch does NOT build a reqwest client / resolve_to_addrs.
        let json_body = serde_json::to_vec(&request_body)
            .context("failed to serialize Trace Commons upload claim request body")?;
        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let response = sink
            .execute(ContributionHttpRequest {
                method: ContributionHttpMethod::Post,
                url: parsed.to_string(),
                bearer_token: issuer_bearer,
                json_body: Some(json_body),
                response_body_limit: TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES as u64,
                timeout_ms,
            })
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "failed to fetch Trace Commons upload claim from {}: {}",
                    safe_trace_upload_claim_issuer_url_label(&parsed),
                    error
                )
            })?;
        (
            response.status,
            bounded_utf8_from_egress_body(response.body),
        )
    } else {
        // WORKER/CLI/TEST path: existing crate-local hardened reqwest client,
        // unchanged behavior (pinned DNS, bounded body, no redirects).
        let host = parsed
            .host_str()
            .ok_or_else(|| {
                anyhow::anyhow!("Trace Commons upload token issuer URL requires a host")
            })?
            .to_ascii_lowercase();
        let port = parsed.port_or_known_default().ok_or_else(|| {
            anyhow::anyhow!("Trace Commons upload token issuer URL requires a known port")
        })?;
        let resolved_addrs = resolve_trace_upload_claim_issuer_host(&host, port).await?;
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(3)))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("ironclaw-trace-commons-upload-claim/0.1")
            .resolve_to_addrs(&host, &resolved_addrs)
            .build()
            .context("failed to build Trace Commons upload token issuer HTTP client")?;
        let mut request = client
            .post(parsed.clone())
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&request_body);
        if let Some(bearer) = issuer_bearer {
            request = request.bearer_auth(bearer);
        }

        let response = request.send().await.with_context(|| {
            format!(
                "failed to fetch Trace Commons upload claim from {}",
                safe_trace_upload_claim_issuer_url_label(&parsed)
            )
        })?;
        let status = response.status();
        if !status.is_success() {
            // Read the (tiny) error body so the typed-label path below can
            // surface a clear diagnostic; bounded by the shared reader.
            let body_text = read_bounded_trace_upload_claim_response(response, &parsed)
                .await
                .unwrap_or_default();
            (status.as_u16(), body_text)
        } else {
            if let Some(content_length) = response.content_length() {
                anyhow::ensure!(
                    content_length <= TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES as u64,
                    "Trace Commons upload claim response from {} exceeded {} bytes",
                    safe_trace_upload_claim_issuer_url_label(&parsed),
                    TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES
                );
            }
            let body_text = read_bounded_trace_upload_claim_response(response, &parsed).await?;
            (status.as_u16(), body_text)
        }
    };

    // Shared handling: status check + typed-label error, JSON parse, validation.
    if !(200..300).contains(&status) {
        return Err(build_trace_upload_claim_http_error(
            &safe_trace_upload_claim_issuer_url_label(&parsed),
            status,
            &body_text,
        ));
    }
    let claim: TraceUploadClaimIssuerResponse =
        serde_json::from_str(&body_text).with_context(|| {
            format!(
                "Trace Commons upload claim response from {} was not valid JSON",
                safe_trace_upload_claim_issuer_url_label(&parsed)
            )
        })?;
    validate_trace_upload_claim_response(&claim)?;
    Ok(claim)
}

pub(crate) async fn read_bounded_trace_upload_claim_response(
    mut response: reqwest::Response,
    issuer_url: &reqwest::Url,
) -> anyhow::Result<String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.with_context(|| {
        format!(
            "failed to read Trace Commons upload claim response from {}",
            safe_trace_upload_claim_issuer_url_label(issuer_url)
        )
    })? {
        // Check the cap BEFORE growing the buffer so an oversized chunk can't
        // push `bytes` past the hard ceiling before the error returns.
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| anyhow::anyhow!("Trace Commons upload claim response size overflow"))?;
        anyhow::ensure!(
            next_len <= TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES,
            "Trace Commons upload claim response from {} exceeded {} bytes",
            safe_trace_upload_claim_issuer_url_label(issuer_url),
            TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES
        );
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).with_context(|| {
        format!(
            "Trace Commons upload claim response from {} was not valid UTF-8",
            safe_trace_upload_claim_issuer_url_label(issuer_url)
        )
    })
}

/// Read an account-traces list response body with a hard byte ceiling so the
/// direct path cannot buffer an unbounded body even when the server omits a
/// `Content-Length` (chunked transfer). Mirrors
/// [`read_bounded_trace_upload_claim_response`] with the larger
/// [`ACCOUNT_TRACES_MAX_RESPONSE_BYTES`] cap.
pub(crate) async fn read_bounded_account_traces_response(
    mut response: reqwest::Response,
) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| anyhow::anyhow!("failed to read account traces response body: {e}"))?
    {
        // Check the cap BEFORE growing the buffer so an oversized chunk can't
        // push `bytes` past the hard ceiling before the error returns.
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| anyhow::anyhow!("account traces response size overflow"))?;
        anyhow::ensure!(
            next_len <= ACCOUNT_TRACES_MAX_RESPONSE_BYTES,
            "account traces response exceeded {} bytes",
            ACCOUNT_TRACES_MAX_RESPONSE_BYTES
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Parse the issuer's typed error label out of an error response body.
/// The issuer returns `{"error": "<Label>"}` for refusals (see
/// trace-commons-server `IssuerError::into_response`). Returns `None`
/// when the body is empty, not JSON, or doesn't carry an `error` string —
/// the caller falls back to a generic HTTP-status diagnostic in that case.
pub(crate) fn parse_trace_upload_claim_error_label(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    parsed
        .get("error")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn validate_trace_upload_claim_response(
    response: &TraceUploadClaimIssuerResponse,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !response.access_token.trim().is_empty(),
        "Trace Commons upload claim response did not include an access token"
    );
    if let Some(token_type) = response.token_type.as_deref() {
        anyhow::ensure!(
            token_type.eq_ignore_ascii_case("bearer"),
            "Trace Commons upload claim response token_type must be bearer"
        );
    }
    let header = jsonwebtoken::decode_header(response.access_token.trim())
        .context("Trace Commons upload claim access token is not a JWT")?;
    anyhow::ensure!(
        header.alg == jsonwebtoken::Algorithm::EdDSA,
        "Trace Commons upload claim access token must use EdDSA"
    );
    anyhow::ensure!(
        header
            .kid
            .as_deref()
            .is_some_and(|kid| !kid.trim().is_empty()),
        "Trace Commons upload claim access token must include a kid"
    );
    Ok(())
}

pub(crate) fn validate_trace_upload_claim_issuer_url(
    url: &reqwest::Url,
    allowed_hosts: &BTreeSet<String>,
) -> anyhow::Result<()> {
    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| anyhow::anyhow!("Trace Commons upload token issuer URL requires a host"))?;
    // Literal-loopback hosts get the same dev exception as the loopback-HTTP
    // invite form in onboarding — otherwise a successful loopback onboarding
    // writes a policy whose claim endpoint can never be used.
    let loopback_dev = crate::onboarding::invite::is_loopback_host(&host);
    anyhow::ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback_dev),
        "Trace Commons upload token issuer URL must use https (or http to a loopback host for standalone)"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Trace Commons upload token issuer URL must not include embedded credentials"
    );
    anyhow::ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "Trace Commons upload token issuer URL must not include query strings or fragments"
    );
    if !loopback_dev {
        anyhow::ensure!(
            !is_internal_trace_upload_claim_issuer_hostname(&host),
            "Trace Commons upload token issuer URL must not use localhost or internal hostnames"
        );
        if let Ok(ip) = host.parse::<IpAddr>() {
            anyhow::ensure!(
                !is_disallowed_trace_upload_claim_issuer_ip(ip),
                "Trace Commons upload token issuer URL must not use private, local, or reserved IP addresses"
            );
        }
    }
    anyhow::ensure!(
        !allowed_hosts.is_empty(),
        "Trace Commons upload token issuer URL requires an allowed-host list"
    );
    anyhow::ensure!(
        allowed_hosts.contains(&host),
        "Trace Commons upload token issuer URL host is not allowlisted"
    );
    Ok(())
}

pub(crate) async fn resolve_trace_upload_claim_issuer_host(
    host: &str,
    port: u16,
) -> anyhow::Result<Vec<SocketAddr>> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .with_context(|| {
            format!("failed to resolve Trace Commons upload token issuer host {host}")
        })?
        .collect();
    anyhow::ensure!(
        !addrs.is_empty(),
        "Trace Commons upload token issuer host {host} resolved to no addresses"
    );
    // For a literal-loopback host (the standalone exception) the pinned
    // resolution must stay on loopback — anything else is DNS tampering.
    let loopback_dev = crate::onboarding::invite::is_loopback_host(host);
    for addr in &addrs {
        if loopback_dev {
            anyhow::ensure!(
                addr.ip().is_loopback(),
                "Trace Commons upload token issuer loopback host {host} resolved to non-loopback address"
            );
            continue;
        }
        anyhow::ensure!(
            !is_disallowed_trace_upload_claim_issuer_ip(addr.ip()),
            "Trace Commons upload token issuer host {host} resolved to disallowed address"
        );
    }
    Ok(addrs)
}

pub(crate) fn is_internal_trace_upload_claim_issuer_hostname(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host == "metadata.google.internal"
}

pub(crate) fn is_disallowed_trace_upload_claim_issuer_ip(ip: IpAddr) -> bool {
    match normalize_trace_upload_claim_issuer_ip(ip) {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            octets[0] == 0
                || octets[0] == 10
                || octets[0] == 127
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 169 && octets[1] == 254)
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 198 && (18..=19).contains(&octets[1]))
                || octets[0] >= 224
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || (segments[0] & 0xff00) == 0xff00
        }
    }
}

pub(crate) fn normalize_trace_upload_claim_issuer_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
    }
}

pub(crate) fn trace_upload_claim_issuer_timeout(
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<Duration> {
    let timeout_ms = policy.upload_token_issuer_timeout_ms;
    anyhow::ensure!(
        (1..=30_000).contains(&timeout_ms),
        "Trace Commons upload token issuer timeout must be between 1 and 30000 milliseconds"
    );
    Ok(Duration::from_millis(timeout_ms))
}

pub(crate) fn safe_trace_upload_claim_issuer_url_label(url: &reqwest::Url) -> String {
    let host = url.host_str().unwrap_or("<unknown-host>");
    format!("{}://{}", url.scheme(), host)
}
