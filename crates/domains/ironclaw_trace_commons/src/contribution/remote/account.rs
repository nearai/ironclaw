//! Trace Commons account APIs: one-time browser login links and the
//! submitted-trace listing.

use anyhow::Context;
use ironclaw_host_api::ids::{TenantId, UserId};

use crate::contribution::*;

// ── Trace Commons account login links ────────────────────────────────────────

/// A one-time browser login link that lands the contributor in their Trace
/// Commons account.
#[derive(Debug, Clone)]
pub struct AccountLoginLink {
    /// The Trace Commons account identifier the link is scoped to.
    pub account_id: String,
    /// The one-time login URL; typically an `/account/login?code=…` path.
    pub url: String,
}

/// Typed classification of an account login-link failure. The host maps these
/// variants to the user-facing `error_code` contract, so that contract no
/// longer depends on substring-matching upstream error wording. The mint path
/// returns the specific variant at each failure site.
#[derive(Debug, thiserror::Error)]
pub enum AccountLoginLinkError {
    /// No enrollment (personal invite or instance) resolved for the caller.
    #[error("not enrolled in Trace Commons")]
    NotEnrolled,
    /// The local enrollment policy could not be read or parsed.
    #[error("could not read Trace Commons enrollment policy")]
    PolicyRead(#[source] anyhow::Error),
    /// Enrollment is incomplete — the upload-claim issuer URL or the local
    /// device-key state is missing/invalid (both surface from the bearer mint).
    #[error("Trace Commons enrollment is incomplete (issuer URL or device-key state)")]
    EnrollmentIncomplete(#[source] anyhow::Error),
    /// The issuer refused to mint the login link (non-2xx HTTP response).
    #[error("Trace Commons issuer refused the login-link request (HTTP {status})")]
    IssuerRefused { status: u16 },
    /// Any other failure — transport, serialization, or a malformed response.
    #[error("Trace Commons login-link request failed")]
    Backend(#[source] anyhow::Error),
    /// The host could not persist the minted link to local state (host-side
    /// write failure; carried here so the host maps one typed contract).
    #[error("could not write the account login link to local state")]
    LocalStateWrite,
}

/// Typed classification of a profile-attribution token mint failure, shared by
/// the `profile_token` and `profile_set` flows (both mint the same token). The
/// host maps these variants to the user-facing `error_code` contract, so it no
/// longer substring-matches upstream error wording.
#[derive(Debug, thiserror::Error)]
pub enum ProfileAttributionError {
    /// No enrollment (personal invite or instance) resolved for the caller.
    #[error("not enrolled in Trace Commons")]
    NotEnrolled,
    /// The local enrollment policy could not be read or parsed.
    #[error("could not read Trace Commons enrollment policy")]
    PolicyRead(#[source] anyhow::Error),
    /// Enrollment is incomplete — the upload-claim issuer URL or the local
    /// device-key state is missing/invalid (both surface from the token mint).
    #[error("Trace Commons enrollment is incomplete (issuer URL or device-key state)")]
    EnrollmentIncomplete(#[source] anyhow::Error),
    /// Any other failure — transport, serialization, or a rejected request.
    #[error("Trace Commons profile request failed")]
    Backend(#[source] anyhow::Error),
    /// The host could not persist minted state locally (host-side write).
    #[error("could not write the profile token to local state")]
    LocalStateWrite,
}

/// Typed classification of a community-profile publish failure: either the
/// caller-supplied handle/bio is invalid, or the underlying attribution mint /
/// request failed (see [`ProfileAttributionError`]).
#[derive(Debug, thiserror::Error)]
pub enum CommunityProfileError {
    /// The display handle or bio failed validation.
    #[error("invalid community profile: {0}")]
    InvalidProfile(String),
    /// The attribution token mint or the profile request failed.
    #[error(transparent)]
    Attribution(#[from] ProfileAttributionError),
}

/// Extract the API base URL (origin) from the configured upload-claim issuer
/// URL by stripping the `/v1/trace-upload-claim` suffix. Other account API
/// endpoints (`/v1/account/login-links`, `/v1/account/traces`, …) are built on
/// top of this shared origin so the derivation is not duplicated.
pub(crate) fn account_api_base_url(
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<String> {
    let issuer_url = policy.upload_token_issuer_url.as_deref().ok_or_else(|| {
        anyhow::anyhow!("Trace Commons upload token issuer URL is not configured")
    })?;
    let base = issuer_url
        .trim_end_matches('/')
        .strip_suffix("/v1/trace-upload-claim")
        .ok_or_else(|| {
            anyhow::anyhow!(
                "upload_token_issuer_url does not end in /v1/trace-upload-claim: {issuer_url}"
            )
        })?;
    Ok(base.to_string())
}

/// Derive the account-login-links URL from the configured upload-claim issuer
/// URL. The login-links service lives at the same origin as the issuer; only
/// the path differs: strip `/v1/trace-upload-claim`, append
/// `/v1/account/login-links`.
pub(crate) fn account_login_links_url(
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<String> {
    Ok(format!(
        "{}/v1/account/login-links",
        account_api_base_url(policy)?
    ))
}

/// Derive the account-traces URL from the configured upload-claim issuer URL.
/// Strip `/v1/trace-upload-claim`, append `/v1/account/traces`.
pub(crate) fn account_traces_url(
    policy: &StandingTraceContributionPolicy,
    limit: Option<usize>,
) -> anyhow::Result<String> {
    let base = account_api_base_url(policy)?;
    // Always send a bounded limit: `None` defaults to ACCOUNT_TRACES_DEFAULT_LIMIT
    // and any explicit value is clamped to [1, ACCOUNT_TRACES_MAX_LIMIT], so no
    // caller can trigger an unbounded server-side history fetch.
    let effective = limit
        .unwrap_or(ACCOUNT_TRACES_DEFAULT_LIMIT)
        .clamp(1, ACCOUNT_TRACES_MAX_LIMIT);
    Ok(format!("{base}/v1/account/traces?limit={effective}"))
}

/// Mint a one-time account login link for the given `(tenant_id, user_id)`.
/// Routes the POST through the caller-supplied `sink` (host egress on the
/// agent path) so the request obeys the deployment's network-egress policy.
///
/// - Resolves the user's Trace Commons credentials; returns an error if the
///   user is not enrolled.
/// - Mints the per-user bearer via `DefaultTraceUploadCredentialProvider`
///   (identical to how submission and profile-attribution flows do it).
/// - POSTs `{ "subject": <subject> }` (field omitted when `subject` is
///   `None`, i.e. personal-invite enrollment) to `/v1/account/login-links`.
/// - Parses the `{ account_id, url }` response into [`AccountLoginLink`].
pub async fn mint_account_login_link_via_sink(
    tenant_id: &TenantId,
    user_id: &UserId,
    sink: &dyn ContributionHttpSink,
) -> Result<AccountLoginLink, AccountLoginLinkError> {
    // Typed at the public boundary so callers can't transpose tenant/user;
    // stringify only when handing off to the dir-parameterised core.
    mint_account_login_link_inner(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id.as_str(),
        user_id.as_str(),
        sink,
    )
    .await
}

/// Direct (non-agent) counterpart to [`mint_account_login_link_via_sink`]
/// for WebUI services and other trusted product surfaces: mints the one-time
/// login link through the [`DirectPinnedContributionSink`] (pinned DNS,
/// private-IP filtering) instead of a host-egress sink.
///
/// Delivery contract: the link is returned ONLY in the result — it is never
/// persisted to a local delivery file. Hosted multi-tenant users cannot read
/// host files; the caller (an authenticated WebUI response) is the delivery
/// channel. The URL must never be logged or placed on any model-visible
/// surface.
pub async fn mint_account_login_link(
    tenant_id: &TenantId,
    user_id: &UserId,
) -> Result<AccountLoginLink, AccountLoginLinkError> {
    // Typed at the public boundary so callers can't transpose tenant/user;
    // stringify only when handing off to the dir-parameterised core.
    mint_account_login_link_direct(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id.as_str(),
        user_id.as_str(),
    )
    .await
}

/// Dir-parameterised core for [`mint_account_login_link`] (direct path).
/// Accepts an explicit `base_dir` so tests can supply an isolated tempdir.
pub(crate) async fn mint_account_login_link_direct(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
) -> Result<AccountLoginLink, AccountLoginLinkError> {
    mint_account_login_link_inner(base_dir, tenant_id, user_id, &DirectPinnedContributionSink).await
}

/// Dir-parameterised core for [`mint_account_login_link_via_sink`].
/// Accepts an explicit `base_dir` so tests can supply an isolated tempdir.
pub(crate) async fn mint_account_login_link_inner(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
    sink: &dyn ContributionHttpSink,
) -> Result<AccountLoginLink, AccountLoginLinkError> {
    let resolution = resolve_trace_credentials_at(base_dir, tenant_id, user_id)
        .map_err(AccountLoginLinkError::PolicyRead)?
        .ok_or(AccountLoginLinkError::NotEnrolled)?;

    // Device key location depends on enrollment type:
    // - Instance enrollment (`subject` is `Some`): the shared device key is at
    //   the instance scope dir (None scope).
    // - Personal-invite enrollment (`subject` is `None`): the user's device key
    //   is at the user scope dir.
    let scope_dir = if resolution.subject.is_some() {
        trace_contribution_dir_for_scope_at(base_dir, None)
    } else {
        trace_contribution_dir_for_scope_at(base_dir, Some(resolution.state_scope.as_str()))
    };

    // Local preconditions FIRST, before any secret/egress work, so incomplete
    // enrollment fails closed with no side effects: a missing issuer URL and a
    // malformed login-links URL are both EnrollmentIncomplete; the claim mint's
    // transport/status/device-key failures below are Backend.
    if upload_claim_issuer_missing(&resolution.policy) {
        return Err(AccountLoginLinkError::EnrollmentIncomplete(
            anyhow::anyhow!("Trace Commons upload-claim issuer URL is not configured"),
        ));
    }
    let url = account_login_links_url(&resolution.policy)
        .map_err(AccountLoginLinkError::EnrollmentIncomplete)?;
    // Parsed once up front: the join base for a relative `url` in the response
    // (its origin is the trust-anchored issuer origin).
    let endpoint_url = reqwest::Url::parse(&url).map_err(|e| {
        AccountLoginLinkError::EnrollmentIncomplete(
            anyhow::Error::new(e).context("login-links URL is not a valid URL"),
        )
    })?;
    let context =
        TraceUploadClaimContext::for_account(resolution.subject.clone()).with_scope_dir(scope_dir);
    // Mint the bearer THROUGH the sink: on the agent path the upload-claim
    // issuer request must route via host RuntimeHttpEgress like the login-link
    // POST below, not the direct reqwest path.
    let bearer = trace_upload_bearer_token_via(&resolution.policy, &context, false, Some(sink))
        .await
        .map_err(AccountLoginLinkError::Backend)?;
    let body = match &resolution.subject {
        Some(s) => serde_json::json!({ "subject": s }),
        None => serde_json::json!({}),
    };
    let body_bytes = serde_json::to_vec(&body).map_err(|e| {
        AccountLoginLinkError::Backend(anyhow::Error::new(e).context("serialize login-link body"))
    })?;
    // Honor the operator-tuned issuer timeout rather than a hardcoded value,
    // matching `execute_community_profile_request`.
    let timeout = trace_upload_claim_issuer_timeout(&resolution.policy)
        .map_err(AccountLoginLinkError::Backend)?;
    let response = sink
        .execute(ContributionHttpRequest {
            method: ContributionHttpMethod::Post,
            url,
            bearer_token: Some(bearer),
            json_body: Some(body_bytes),
            response_body_limit: TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES as u64,
            timeout_ms: u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX),
        })
        .await
        .map_err(|e| {
            AccountLoginLinkError::Backend(anyhow::anyhow!("login-link request failed: {e}"))
        })?;
    if !(200..300).contains(&response.status) {
        return Err(AccountLoginLinkError::IssuerRefused {
            status: response.status,
        });
    }
    let parsed: serde_json::Value = serde_json::from_slice(&response.body).map_err(|e| {
        AccountLoginLinkError::Backend(anyhow::Error::new(e).context("login-link response JSON"))
    })?;
    let account_id = parsed["account_id"]
        .as_str()
        .ok_or_else(|| {
            AccountLoginLinkError::Backend(anyhow::anyhow!(
                "login-link response missing account_id"
            ))
        })?
        .to_string();
    let link_url = parsed["url"]
        .as_str()
        .ok_or_else(|| {
            AccountLoginLinkError::Backend(anyhow::anyhow!("login-link response missing url"))
        })?
        .to_string();
    // The server may return a relative path (e.g. `/account/login?code=…`).
    // Resolve it against the login-links endpoint — whose origin is the
    // trust-anchored issuer origin — so every delivery channel (browser
    // navigation, local delivery file) receives an absolute URL instead of
    // one that would resolve against the WRONG origin (e.g. the IronClaw
    // WebUI's own host).
    let resolved = match reqwest::Url::parse(&link_url) {
        Ok(absolute) => absolute,
        Err(_) => endpoint_url.join(&link_url).map_err(|e| {
            AccountLoginLinkError::Backend(
                anyhow::Error::new(e).context("login-link response url is not resolvable"),
            )
        })?,
    };
    // ORIGIN PIN: the caller navigates an authenticated user's browser to this
    // URL. A hostile or compromised issuer response must not be able to steer
    // that navigation anywhere else — the final URL must stay on the
    // trust-anchored issuer origin (same scheme + host + port as the
    // login-links endpoint; this also excludes non-HTTP(S) schemes such as
    // `javascript:`) and must carry no userinfo.
    let same_origin = resolved.scheme() == endpoint_url.scheme()
        && resolved.host_str() == endpoint_url.host_str()
        && resolved.port_or_known_default() == endpoint_url.port_or_known_default();
    if !same_origin || !resolved.username().is_empty() || resolved.password().is_some() {
        return Err(AccountLoginLinkError::Backend(anyhow::anyhow!(
            "login-link response url is not on the issuer origin"
        )));
    }
    Ok(AccountLoginLink {
        account_id,
        url: resolved.to_string(),
    })
}

// ── Trace Commons account traces ──────────────────────────────────────────────

/// A single submitted trace record as returned by `GET /v1/account/traces`.
/// Only the fields the UI needs are projected here; unknown server fields are
/// ignored via `#[serde(default)]` and `#[serde(deny_unknown_fields)]` is
/// deliberately omitted.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AccountTraceItem {
    #[serde(default)]
    pub submission_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub credit_points_pending: f32,
    #[serde(default)]
    pub credit_points_final: Option<f32>,
    #[serde(default)]
    pub received_at: Option<String>,
}

/// Agent-path (host-egress sink) counterpart to the direct `fetch_account_traces`.
/// Not yet wired to a first-party capability — retained as the sink-based entry
/// for a future `trace_commons.account_traces` agent capability, mirroring
/// `mint_account_login_link_via_sink`. Covered by unit tests.
///
/// Fetch the list of submitted traces for the given `(tenant_id, user_id)` via
/// the caller-supplied `sink` (host egress on the agent path).
///
/// - Resolves the user's Trace Commons credentials; returns `Ok(vec![])` when
///   the user is not enrolled (lenient zero-state, not an error).
/// - Mints the per-user bearer via `DefaultTraceUploadCredentialProvider`
///   (identical to how submission and profile-attribution flows do it).
/// - GETs `<origin>/v1/account/traces?limit=N` and parses the JSON array into
///   `Vec<AccountTraceItem>`. Non-2xx for an unenrolled/empty case also
///   returns `Ok(vec![])`. Transport failures return `Err`.
pub async fn fetch_account_traces_via_sink(
    tenant_id: &str,
    user_id: &str,
    limit: Option<usize>,
    sink: &dyn ContributionHttpSink,
) -> anyhow::Result<Vec<AccountTraceItem>> {
    fetch_account_traces_inner(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id,
        user_id,
        limit,
        sink,
    )
    .await
}

/// Fetch the list of submitted traces for the given `(tenant_id, user_id)` using
/// the crate-local hardened reqwest client (the direct/CLI path, no host-egress
/// sink required).
///
/// This is the service-safe counterpart to [`fetch_account_traces_via_sink`]: it
/// uses the [`pinned_trace_commons_http_client`] (private-IP-filtered, pinned
/// DNS resolution — the same hardening as the upload-claim issuer request), so
/// a rebinding host cannot redirect this bearer-authenticated GET to an
/// internal address, without coupling the caller to a host-egress
/// `ContributionHttpSink`. Use this from WebUI services and any non-agent
/// surface. Use [`fetch_account_traces_via_sink`] from the agent runtime where
/// all egress must flow through `RuntimeHttpEgress`.
///
/// Returns `Ok(vec![])` when the user is not enrolled, or when the server
/// returns 404 (an enrolled principal with no account/traces yet). Any other
/// non-2xx status and all transport failures return `Err`.
pub async fn fetch_account_traces(
    tenant_id: &str,
    user_id: &str,
    limit: Option<usize>,
) -> anyhow::Result<Vec<AccountTraceItem>> {
    fetch_account_traces_direct(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id,
        user_id,
        limit,
    )
    .await
}

/// Dir-parameterised core for [`fetch_account_traces`] (direct/CLI path).
/// Accepts an explicit `base_dir` so tests can supply an isolated tempdir.
pub(crate) async fn fetch_account_traces_direct(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
    limit: Option<usize>,
) -> anyhow::Result<Vec<AccountTraceItem>> {
    let resolution = match resolve_trace_credentials_at(base_dir, tenant_id, user_id)? {
        Some(r) => r,
        None => return Ok(vec![]),
    };

    let scope_dir = if resolution.subject.is_some() {
        trace_contribution_dir_for_scope_at(base_dir, None)
    } else {
        trace_contribution_dir_for_scope_at(base_dir, Some(resolution.state_scope.as_str()))
    };

    let context =
        TraceUploadClaimContext::for_account(resolution.subject.clone()).with_scope_dir(scope_dir);
    let provider = DefaultTraceUploadCredentialProvider;
    let bearer = provider
        .bearer_token(&resolution.policy, &context, false)
        .await?;
    let url = account_traces_url(&resolution.policy, limit)?;
    let url = reqwest::Url::parse(&url).context("account traces URL is not a valid URL")?;
    // Pinned-DNS, private-IP-filtered client: the bearer minted above must not
    // be attachable to an internal address via DNS rebinding between the claim
    // request and this GET.
    let client =
        pinned_trace_commons_http_client(&resolution.policy, &url, "ironclaw-trace-commons-client")
            .await?;
    let response = client
        .get(url)
        .bearer_auth(&bearer)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("account traces request failed: {e}"))?;
    let status = response.status();
    // 404 means this enrolled principal has no account/traces yet — a legitimate
    // empty state. Any OTHER non-2xx (401/403/429/5xx) is a real failure and must
    // surface as an error so the WebUI boundary renders a sanitized unavailable
    // state rather than a misleading "no traces".
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(vec![]);
    }
    anyhow::ensure!(
        status.is_success(),
        "account traces request returned status {}",
        status.as_u16()
    );
    let body = read_bounded_account_traces_response(response).await?;
    let items: Vec<AccountTraceItem> = serde_json::from_slice(&body)
        .context("account traces response was not a valid JSON array")?;
    Ok(items)
}

/// Dir-parameterised core for [`fetch_account_traces_via_sink`].
/// Accepts an explicit `base_dir` so tests can supply an isolated tempdir.
pub(crate) async fn fetch_account_traces_inner(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
    limit: Option<usize>,
    sink: &dyn ContributionHttpSink,
) -> anyhow::Result<Vec<AccountTraceItem>> {
    let resolution = match resolve_trace_credentials_at(base_dir, tenant_id, user_id)? {
        Some(r) => r,
        None => return Ok(vec![]),
    };

    let scope_dir = if resolution.subject.is_some() {
        trace_contribution_dir_for_scope_at(base_dir, None)
    } else {
        trace_contribution_dir_for_scope_at(base_dir, Some(resolution.state_scope.as_str()))
    };

    let context =
        TraceUploadClaimContext::for_account(resolution.subject.clone()).with_scope_dir(scope_dir);
    // Mint the bearer THROUGH the sink: on the agent path the upload-claim
    // issuer request must route via host RuntimeHttpEgress like the traces
    // GET below, not the direct reqwest path.
    let bearer =
        trace_upload_bearer_token_via(&resolution.policy, &context, false, Some(sink)).await?;
    let url = account_traces_url(&resolution.policy, limit)?;
    // Honor the operator-tuned issuer timeout rather than a hardcoded value,
    // and cap the body at the account-traces ceiling (a legitimate trace list
    // can exceed the smaller claim-response cap the mint paths use).
    let timeout = trace_upload_claim_issuer_timeout(&resolution.policy)?;
    let response = sink
        .execute(ContributionHttpRequest {
            method: ContributionHttpMethod::Get,
            url,
            bearer_token: Some(bearer),
            json_body: None,
            response_body_limit: ACCOUNT_TRACES_MAX_RESPONSE_BYTES as u64,
            timeout_ms: u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX),
        })
        .await
        .map_err(|e| anyhow::anyhow!("account traces request failed: {e}"))?;
    // 404 = no account/traces yet for this enrolled principal (legitimate empty);
    // any other non-2xx is a real failure and propagates as an error. The host
    // egress already bounded the body via `response_body_limit` above.
    if response.status == 404 {
        return Ok(vec![]);
    }
    anyhow::ensure!(
        (200..300).contains(&response.status),
        "account traces request returned status {}",
        response.status
    );
    let items: Vec<AccountTraceItem> = serde_json::from_slice(&response.body)
        .context("account traces response was not a valid JSON array")?;
    Ok(items)
}
