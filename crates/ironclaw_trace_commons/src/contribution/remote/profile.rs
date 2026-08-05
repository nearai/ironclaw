//! The Trace Commons community profile: attribution-token minting, handle
//! and bio validation, and profile set/withdraw.

use anyhow::Context;
use chrono::{DateTime, Utc};
use ironclaw_host_api::ids::{TenantId, UserId};
use serde_json::Value;
use std::net::IpAddr;
use std::time::Duration;

use crate::contribution::*;

pub const COMMUNITY_PROFILE_HANDLE_MIN_CHARS: usize = 3;
pub const COMMUNITY_PROFILE_HANDLE_MAX_CHARS: usize = 32;
pub const COMMUNITY_PROFILE_BIO_MAX_BYTES: usize = 280;
pub(crate) const COMMUNITY_PROFILE_PATH: &str = "/v1/community/profile";

/// Short-lived claim minted from the upload-claim issuer that authorizes
/// community-profile management only (consent scope `public_attribution`,
/// empty allowed-uses). A claim scoped to only `public_attribution` cannot
/// submit traces — it gates the `/v1/community/profile` endpoints.
#[derive(Debug, Clone)]
pub struct ProfileAttributionToken {
    pub access_token: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub expires_in: Option<i64>,
}

/// Claim context for the community-profile second opt-in: no trace ids, the
/// `public_attribution` consent scope only, and no allowed uses.
pub(crate) fn profile_attribution_claim_context(scope: Option<&str>) -> TraceUploadClaimContext {
    TraceUploadClaimContext {
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![ConsentScope::PublicAttribution],
        allowed_uses: Vec::new(),
        scope_dir: Some(trace_contribution_dir_for_scope(scope)),
        subject: None,
    }
}

/// Build a profile-attribution claim context from an instance-aware
/// [`TraceCredentialResolution`]. The device key lives at the instance scope
/// dir when a pseudonymous `subject` is present (instance enrollment) and at the
/// user scope dir otherwise (personal-invite enrollment) — the same scope_dir /
/// subject selection as `mint_account_login_link_inner`.
pub(crate) fn profile_attribution_claim_context_from_resolution(
    base_dir: &std::path::Path,
    resolution: &TraceCredentialResolution,
) -> TraceUploadClaimContext {
    let scope_dir = if resolution.subject.is_some() {
        trace_contribution_dir_for_scope_at(base_dir, None)
    } else {
        trace_contribution_dir_for_scope_at(base_dir, Some(resolution.state_scope.as_str()))
    };
    TraceUploadClaimContext {
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![ConsentScope::PublicAttribution],
        allowed_uses: Vec::new(),
        scope_dir: Some(scope_dir),
        subject: resolution.subject.clone(),
    }
}

/// True when the resolved enrollment is missing the upload-claim issuer URL — a
/// local *precondition* failure (enrollment incomplete), distinct from the
/// transport/backend failures that surface later from the claim mint. Callers
/// check this so a missing URL maps to `EnrollmentIncomplete` while post-check
/// failures map to `Backend`, instead of collapsing everything into one code.
pub(crate) fn upload_claim_issuer_missing(policy: &StandingTraceContributionPolicy) -> bool {
    policy
        .upload_token_issuer_url
        .as_deref()
        .is_none_or(|url| url.trim().is_empty())
}

/// Mint a short-lived profile-attribution token from the configured Trace
/// Commons upload-claim issuer. The token authorizes community-profile
/// management only and cannot submit traces.
pub async fn mint_profile_attribution_token_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<ProfileAttributionToken> {
    let policy = read_trace_policy_for_scope(scope)?;
    mint_profile_attribution_token_with_policy(&policy, scope, None).await
}

/// Agent-invoked variant: routes the upload-claim mint through the host
/// `RuntimeHttpEgress` pipeline via the injected `sink`.
pub async fn mint_profile_attribution_token_for_scope_via_sink(
    scope: Option<&str>,
    sink: &dyn ContributionHttpSink,
) -> anyhow::Result<ProfileAttributionToken> {
    let policy = read_trace_policy_for_scope(scope)?;
    mint_profile_attribution_token_with_policy(&policy, scope, Some(sink)).await
}

/// Instance-aware variant of [`mint_profile_attribution_token_for_scope_via_sink`]:
/// resolves the caller's enrollment (personal invite OR admin-provisioned
/// instance enrollment) via [`resolve_trace_credentials`], so an instance-only
/// contributor mints under the shared instance device key with a per-user
/// pseudonymous subject rather than being falsely rejected as not enrolled.
pub async fn mint_profile_attribution_token_for_user_via_sink(
    tenant_id: &TenantId,
    user_id: &UserId,
    sink: &dyn ContributionHttpSink,
) -> Result<ProfileAttributionToken, ProfileAttributionError> {
    // Typed at the public boundary; stringify only for the dir-parameterised core.
    mint_profile_attribution_token_for_user_inner(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id.as_str(),
        user_id.as_str(),
        sink,
    )
    .await
}

/// Dir-parameterised core for [`mint_profile_attribution_token_for_user_via_sink`].
/// Accepts an explicit `base_dir` so tests can supply an isolated tempdir.
pub(crate) async fn mint_profile_attribution_token_for_user_inner(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
    sink: &dyn ContributionHttpSink,
) -> Result<ProfileAttributionToken, ProfileAttributionError> {
    let resolution = resolve_trace_credentials_at(base_dir, tenant_id, user_id)
        .map_err(ProfileAttributionError::PolicyRead)?
        .ok_or(ProfileAttributionError::NotEnrolled)?;
    // Local precondition: a missing issuer URL is EnrollmentIncomplete.
    if upload_claim_issuer_missing(&resolution.policy) {
        return Err(ProfileAttributionError::EnrollmentIncomplete(
            anyhow::anyhow!("Trace Commons upload-claim issuer URL is not configured"),
        ));
    }
    let context = profile_attribution_claim_context_from_resolution(base_dir, &resolution);
    // Post-precondition failures (issuer transport/status, serde, device-key)
    // are Backend — not "re-run onboarding".
    mint_profile_attribution_token_with_context(&resolution.policy, &context, Some(sink))
        .await
        .map_err(ProfileAttributionError::Backend)
}

pub(crate) async fn mint_profile_attribution_token_with_policy(
    policy: &StandingTraceContributionPolicy,
    scope: Option<&str>,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<ProfileAttributionToken> {
    let context = profile_attribution_claim_context(scope);
    mint_profile_attribution_token_with_context(policy, &context, sink).await
}

/// Mint a profile-attribution token using a prebuilt claim context. Shared by
/// the scope-based (`*_for_scope_*`) and instance-aware (`*_for_user_*`) entry
/// points so the enabled/issuer gates and the issuer round-trip stay in one
/// place regardless of how the context (scope_dir + subject) was derived.
pub(crate) async fn mint_profile_attribution_token_with_context(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<ProfileAttributionToken> {
    anyhow::ensure!(
        policy.enabled,
        "not enrolled in Trace Commons — onboard first (run the Trace Commons onboarding \
         or `ironclaw traces opt-in`)"
    );
    anyhow::ensure!(
        policy
            .upload_token_issuer_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty()),
        "Trace Commons upload token issuer URL is not configured; re-run onboarding"
    );
    let claim = fetch_trace_upload_claim_from_issuer(policy, context, sink).await?;
    Ok(ProfileAttributionToken {
        access_token: claim.access_token,
        expires_at: claim.expires_at,
        expires_in: claim.expires_in,
    })
}

/// Create or update the public community profile for this scope. Mints a
/// fresh profile-attribution token and PUTs the profile to the Trace Commons
/// community endpoint derived from the policy's ingest URL.
pub async fn set_community_profile_for_scope(
    scope: Option<&str>,
    display_handle: &str,
    bio: Option<&str>,
) -> anyhow::Result<()> {
    set_community_profile_for_scope_inner(scope, display_handle, bio, None).await
}

/// Agent-invoked variant: routes BOTH the upload-claim mint AND the profile PUT
/// through the host `RuntimeHttpEgress` pipeline via the injected `sink`.
pub async fn set_community_profile_for_scope_via_sink(
    scope: Option<&str>,
    display_handle: &str,
    bio: Option<&str>,
    sink: &dyn ContributionHttpSink,
) -> anyhow::Result<()> {
    set_community_profile_for_scope_inner(scope, display_handle, bio, Some(sink)).await
}

/// Instance-aware variant of [`set_community_profile_for_scope_via_sink`]:
/// resolves the caller's enrollment via [`resolve_trace_credentials`] so an
/// instance-only contributor can publish a community profile under the shared
/// instance device key with a per-user pseudonymous subject.
pub async fn set_community_profile_for_user_via_sink(
    tenant_id: &TenantId,
    user_id: &UserId,
    display_handle: &str,
    bio: Option<&str>,
    sink: &dyn ContributionHttpSink,
) -> Result<(), CommunityProfileError> {
    // Typed at the public boundary; stringify only for the dir-parameterised core.
    set_community_profile_for_user_inner(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id.as_str(),
        user_id.as_str(),
        display_handle,
        bio,
        Some(sink),
    )
    .await
}

/// Dir-parameterised core for [`set_community_profile_for_user_via_sink`].
/// Accepts an explicit `base_dir` so tests can supply an isolated tempdir.
pub(crate) async fn set_community_profile_for_user_inner(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
    display_handle: &str,
    bio: Option<&str>,
    sink: Option<&dyn ContributionHttpSink>,
) -> Result<(), CommunityProfileError> {
    let handle = validate_community_profile_handle(display_handle)
        .map_err(|e| CommunityProfileError::InvalidProfile(format!("{e:#}")))?;
    if let Some(bio) = bio {
        validate_community_profile_bio(bio)
            .map_err(|e| CommunityProfileError::InvalidProfile(format!("{e:#}")))?;
    }
    let resolution = resolve_trace_credentials_at(base_dir, tenant_id, user_id)
        .map_err(ProfileAttributionError::PolicyRead)?
        .ok_or(ProfileAttributionError::NotEnrolled)?;
    // Local preconditions (missing ingest URL or issuer URL) are
    // EnrollmentIncomplete; the mint/PUT transport failures below are Backend.
    let url = community_profile_url_from_policy(&resolution.policy)
        .map_err(ProfileAttributionError::EnrollmentIncomplete)?;
    if upload_claim_issuer_missing(&resolution.policy) {
        return Err(
            ProfileAttributionError::EnrollmentIncomplete(anyhow::anyhow!(
                "Trace Commons upload-claim issuer URL is not configured"
            ))
            .into(),
        );
    }
    let context = profile_attribution_claim_context_from_resolution(base_dir, &resolution);
    let token = mint_profile_attribution_token_with_context(&resolution.policy, &context, sink)
        .await
        .map_err(ProfileAttributionError::Backend)?;
    let body = serde_json::json!({
        "display_handle": handle,
        "bio": bio,
    });
    execute_community_profile_request(
        &resolution.policy,
        ContributionHttpMethod::Put,
        url,
        &token.access_token,
        Some(&body),
        sink,
    )
    .await
    .map_err(ProfileAttributionError::Backend)?;
    Ok(())
}

pub(crate) async fn set_community_profile_for_scope_inner(
    scope: Option<&str>,
    display_handle: &str,
    bio: Option<&str>,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<()> {
    let handle = validate_community_profile_handle(display_handle)?;
    if let Some(bio) = bio {
        validate_community_profile_bio(bio)?;
    }
    let policy = read_trace_policy_for_scope(scope)?;
    let url = community_profile_url_from_policy(&policy)?;
    let token = mint_profile_attribution_token_with_policy(&policy, scope, sink).await?;
    let body = serde_json::json!({
        "display_handle": handle,
        "bio": bio,
    });
    execute_community_profile_request(
        &policy,
        ContributionHttpMethod::Put,
        url,
        &token.access_token,
        Some(&body),
        sink,
    )
    .await
}

/// Withdraw the public community profile for this scope (DELETE the profile
/// resource). Mints a fresh profile-attribution token like
/// [`set_community_profile_for_scope`].
pub async fn withdraw_community_profile_for_scope(scope: Option<&str>) -> anyhow::Result<()> {
    let policy = read_trace_policy_for_scope(scope)?;
    let url = community_profile_url_from_policy(&policy)?;
    let token = mint_profile_attribution_token_with_policy(&policy, scope, None).await?;
    execute_community_profile_request(
        &policy,
        ContributionHttpMethod::Delete,
        url,
        &token.access_token,
        None,
        None,
    )
    .await
}

/// Derive the community-profile endpoint from the policy's ingest endpoint,
/// keeping scheme/host/port and replacing only the path. Profile writes are
/// handled by ingest, while upload-claim minting remains issuer-owned.
pub(crate) fn community_profile_url_from_policy(
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<reqwest::Url> {
    let ingest_url = policy
        .ingestion_endpoint
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("Trace Commons ingest endpoint is not configured; re-run onboarding")
        })?;
    let mut url =
        reqwest::Url::parse(ingest_url).context("invalid Trace Commons ingest endpoint")?;
    validate_trace_commons_ingest_url(&url)?;
    // Preserve any mount prefix on the ingest path (e.g. `/api/v1/traces`),
    // mirroring `trace_submission_status_endpoint`, instead of clobbering the
    // whole path — otherwise a prefixed deployment 404s on profile PUT/DELETE.
    let path = url.path().trim_end_matches('/');
    let new_path = if let Some(prefix) = path.strip_suffix("/v1/traces") {
        format!("{}/v1/community/profile", prefix.trim_end_matches('/'))
    } else if let Some(prefix) = path.strip_suffix("/traces") {
        format!("{}/community/profile", prefix.trim_end_matches('/'))
    } else {
        COMMUNITY_PROFILE_PATH.to_string()
    };
    url.set_path(&new_path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

pub(crate) fn validate_trace_commons_ingest_url(url: &reqwest::Url) -> anyhow::Result<()> {
    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| anyhow::anyhow!("Trace Commons ingest endpoint requires a host"))?;
    // Same literal-loopback dev exception as the claim-issuer validator: a
    // loopback-HTTP invite stores a loopback ingest endpoint in the policy.
    let loopback_dev = crate::onboarding::invite::is_loopback_host(&host);
    anyhow::ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback_dev),
        "Trace Commons ingest endpoint must use https (or http to a loopback host for standalone)"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Trace Commons ingest endpoint must not include embedded credentials"
    );
    anyhow::ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "Trace Commons ingest endpoint must not include query strings or fragments"
    );
    if !loopback_dev {
        anyhow::ensure!(
            !is_internal_trace_upload_claim_issuer_hostname(&host),
            "Trace Commons ingest endpoint must not use localhost or internal hostnames"
        );
        if let Ok(ip) = host.parse::<IpAddr>() {
            anyhow::ensure!(
                !is_disallowed_trace_upload_claim_issuer_ip(ip),
                "Trace Commons ingest endpoint must not use private, local, or reserved IP addresses"
            );
        }
    }
    Ok(())
}

pub(crate) fn validate_community_profile_handle(handle: &str) -> anyhow::Result<String> {
    let trimmed = handle.trim();
    anyhow::ensure!(
        trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "community profile handle may only contain ASCII letters, digits, '-' and '_'"
    );
    // All-ASCII at this point, so byte length == character length.
    anyhow::ensure!(
        trimmed.len() >= COMMUNITY_PROFILE_HANDLE_MIN_CHARS,
        "community profile handle must be at least {COMMUNITY_PROFILE_HANDLE_MIN_CHARS} characters"
    );
    anyhow::ensure!(
        trimmed.len() <= COMMUNITY_PROFILE_HANDLE_MAX_CHARS,
        "community profile handle must be at most {COMMUNITY_PROFILE_HANDLE_MAX_CHARS} characters"
    );
    Ok(trimmed.to_string())
}

pub(crate) fn validate_community_profile_bio(bio: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        bio.len() <= COMMUNITY_PROFILE_BIO_MAX_BYTES,
        "community profile bio must be at most {COMMUNITY_PROFILE_BIO_MAX_BYTES} bytes"
    );
    Ok(())
}

/// Build a hardened HTTP client for Trace Commons account-surface requests:
/// pinned DNS resolution against the validated host (private/internal IPs
/// rejected via `resolve_trace_upload_claim_issuer_host`), policy-derived
/// bounded timeouts, and no redirect following — mirroring
/// `fetch_trace_upload_claim_from_issuer`. The pinned resolution closes the
/// DNS-rebinding window between claim validation and the follow-up request.
pub(crate) async fn pinned_trace_commons_http_client(
    policy: &StandingTraceContributionPolicy,
    url: &reqwest::Url,
    user_agent: &str,
) -> anyhow::Result<reqwest::Client> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("Trace Commons request URL requires a host"))?
        .to_ascii_lowercase();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("Trace Commons request URL requires a known port"))?;
    let resolved_addrs = resolve_trace_upload_claim_issuer_host(&host, port).await?;
    let timeout = trace_upload_claim_issuer_timeout(policy)?;
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(3)))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(user_agent)
        .resolve_to_addrs(&host, &resolved_addrs)
        .build()
        .context("failed to build pinned Trace Commons HTTP client")
}

/// Build the hardened HTTP client for community-profile requests. See
/// [`pinned_trace_commons_http_client`].
pub(crate) async fn community_profile_http_client(
    policy: &StandingTraceContributionPolicy,
    url: &reqwest::Url,
) -> anyhow::Result<reqwest::Client> {
    pinned_trace_commons_http_client(policy, url, "ironclaw-trace-commons-community-profile/0.1")
        .await
}

pub(crate) fn community_profile_method_label(method: ContributionHttpMethod) -> &'static str {
    match method {
        ContributionHttpMethod::Get => "GET",
        ContributionHttpMethod::Post => "POST",
        ContributionHttpMethod::Put => "PUT",
        ContributionHttpMethod::Delete => "DELETE",
    }
}

/// Send a community-profile request and map non-success statuses to a bounded
/// diagnostic. The bearer token and raw response bodies never appear in
/// errors or logs — only the bounded JSON `error` field, when present.
///
/// `sink == Some`: AGENT path — route through the host `RuntimeHttpEgress`
/// pipeline. `sink == None`: WORKER/CLI path — build the crate-local hardened
/// reqwest client via [`community_profile_http_client`] (unchanged behavior).
pub(crate) async fn execute_community_profile_request(
    policy: &StandingTraceContributionPolicy,
    method: ContributionHttpMethod,
    url: reqwest::Url,
    access_token: &str,
    body: Option<&Value>,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<()> {
    let method_label = community_profile_method_label(method);

    let (status, body_text): (u16, String) = if let Some(sink) = sink {
        let json_body = match body {
            Some(body) => Some(
                serde_json::to_vec(body)
                    .context("failed to serialize Trace Commons community profile request body")?,
            ),
            None => None,
        };
        let timeout = trace_upload_claim_issuer_timeout(policy)?;
        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let response = sink
            .execute(ContributionHttpRequest {
                method,
                url: url.to_string(),
                bearer_token: Some(access_token.to_string()),
                json_body,
                response_body_limit: TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES as u64,
                timeout_ms,
            })
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "Trace Commons community profile {} request to {} failed: {}",
                    method_label,
                    safe_trace_upload_claim_issuer_url_label(&url),
                    error
                )
            })?;
        (
            response.status,
            bounded_utf8_from_egress_body(response.body),
        )
    } else {
        let client = community_profile_http_client(policy, &url).await?;
        let reqwest_method = match method {
            ContributionHttpMethod::Get => reqwest::Method::GET,
            ContributionHttpMethod::Post => reqwest::Method::POST,
            ContributionHttpMethod::Put => reqwest::Method::PUT,
            ContributionHttpMethod::Delete => reqwest::Method::DELETE,
        };
        let mut request = client
            .request(reqwest_method, url.clone())
            .header(reqwest::header::ACCEPT, "application/json")
            .bearer_auth(access_token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.with_context(|| {
            format!(
                "Trace Commons community profile {} request to {} failed",
                method_label,
                safe_trace_upload_claim_issuer_url_label(&url)
            )
        })?;
        let status = response.status();
        if status.is_success() {
            (status.as_u16(), String::new())
        } else {
            let body_text = read_bounded_trace_upload_claim_response(response, &url)
                .await
                .unwrap_or_default(); // silent-ok: error-body read best-effort, status alone is diagnostic
            (status.as_u16(), body_text)
        }
    };

    if !(200..300).contains(&status) {
        let label = parse_trace_upload_claim_error_label(&body_text);
        return Err(anyhow::anyhow!(
            "Trace Commons community profile {} request to {} rejected: HTTP {}{}",
            method_label,
            safe_trace_upload_claim_issuer_url_label(&url),
            status,
            label
                .as_deref()
                .map(|l| format!(" ({l})"))
                .unwrap_or_default(),
        ));
    }
    Ok(())
}
