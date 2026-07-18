// arch-exempt: large_file, trace-commons capability family shares one dispatch surface, plan #4539
//! First-party Trace Commons capabilities: onboard, status, credits, profile token, profile set,
//! and account login link.
//!
//! `trace_commons.onboard` drives the operator-invite enrollment flow.
//! `trace_commons.status` is a read-only policy inspector.
//! `trace_commons.credits` is a read-only credit balance reporter.
//! `trace_commons.profile_token` mints a short-lived public-attribution token.
//! `trace_commons.profile_set` updates the public community profile directly.
//! `trace_commons.account_login_link` mints a one-time browser login URL.
//!
//! All six are model-visible.

use std::{panic::AssertUnwindSafe, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use futures_util::FutureExt as _;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    CapabilityId, EffectKind, NetworkMethod, NetworkPolicy, PermissionMode, ResourceEstimate,
    ResourceProfile, ResourceScope, RuntimeCredentialInjection, RuntimeCredentialSource,
    RuntimeCredentialTarget, RuntimeDispatchErrorKind, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeKind, SecretHandle,
};
use ironclaw_reborn_traces::contribution::{
    AccountLoginLink, AccountLoginLinkError, COMMUNITY_PROFILE_BIO_MAX_BYTES,
    COMMUNITY_PROFILE_HANDLE_MAX_CHARS, COMMUNITY_PROFILE_HANDLE_MIN_CHARS, CommunityProfileError,
    ContributionHttpError, ContributionHttpMethod, ContributionHttpRequest,
    ContributionHttpResponse, ContributionHttpSink, ProfileAttributionError,
    ProfileAttributionToken, StandingTraceContributionPolicy, TraceCreditReport,
    TraceUploadAuthMode, mint_account_login_link_via_sink,
    mint_profile_attribution_token_for_user_via_sink, resolve_trace_credentials,
    set_community_profile_for_user_via_sink, trace_contribution_dir_for_scope, trace_scope_key,
};
use ironclaw_reborn_traces::onboarding::{
    OnboardConsents, OnboardError, OnboardHttpResponse, OnboardOutcome, OnboardingHttpSink,
    protocol::OnboardErrorCode,
};
use ironclaw_secrets::SecretMaterial;
use serde_json::{Value, json};

use crate::FirstPartyCapabilityError;
use crate::FirstPartyCapabilityRequest;
use crate::RuntimeSecretMaterialStager;

/// Secret handle under which the host-minted Trace Commons bearer token is
/// staged for one-shot credential injection into the outbound Authorization
/// header. The token is delivered through the staged credential-injection path
/// (stager + `apply_credential_injections`), never as a raw request header, so
/// the egress sensitive-header guard still applies to model-supplied headers.
const TRACE_COMMONS_BEARER_HANDLE: &str = "trace_commons_bearer";

/// Maximum onboarding response body accepted (64 KiB), mirroring the cap the
/// onboarding module enforces for its default sink.
const ONBOARD_MAX_RESPONSE_BODY: u64 = 64 * 1024;
/// Onboarding POST timeout in milliseconds (10s), mirroring the onboarding
/// module's `ONBOARD_TIMEOUT_SECS`.
const ONBOARD_TIMEOUT_MS: u32 = 10_000;

use super::{
    FIRST_PARTY_DEFAULT_OUTPUT_BYTES, first_party_capability_manifest, input_error,
    resource_profile,
};

pub const TRACE_COMMONS_ONBOARD_CAPABILITY_ID: &str = "builtin.trace_commons.onboard";
pub const TRACE_COMMONS_STATUS_CAPABILITY_ID: &str = "builtin.trace_commons.status";
pub const TRACE_COMMONS_CREDITS_CAPABILITY_ID: &str = "builtin.trace_commons.credits";
pub const TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID: &str = "builtin.trace_commons.profile_token";
pub const TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID: &str = "builtin.trace_commons.profile_set";
pub const TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID: &str =
    "builtin.trace_commons.account_login_link";

// ── Manifest helpers ─────────────────────────────────────────────────────────

pub(super) fn onboard_manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
        "Enroll this IronClaw in Trace Commons using an operator-issued invite link. \
         ONLY call after the user has explicitly (1) confirmed they want to contribute \
         redacted traces and (2) chosen whether to include redacted message text and \
         redacted tool payloads. Pass confirmed=true only when both consents were given \
         in this conversation.",
        // Onboarding persists device-key material (reads the standing policy,
        // writes the per-tenant Ed25519 keypair + policy.json), so the effect
        // model must declare the local filesystem read/write, not just the
        // network enrollment POST.
        vec![
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::Network,
            EffectKind::ExternalWrite,
        ],
        PermissionMode::Ask,
        Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(15_000)
                // The surface contract requires every visible capability to
                // advertise an output_bytes estimate.
                .set_output_bytes(FIRST_PARTY_DEFAULT_OUTPUT_BYTES),
            hard_ceiling: None,
        }),
    )
}

pub(super) fn status_manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        TRACE_COMMONS_STATUS_CAPABILITY_ID,
        "Report Trace Commons enrollment state for the current user: enrolled or not, \
         tenant, auth mode, and consent settings.",
        vec![EffectKind::ReadFilesystem],
        PermissionMode::Allow,
        // Reuse the shared default profile (small JSON status output) so the
        // capability advertises an output_bytes estimate like the other reads.
        resource_profile(),
    )
}

pub(super) fn credits_manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        TRACE_COMMONS_CREDITS_CAPABILITY_ID,
        "Report the current user's Trace Commons credit state: pending and final balance, \
         submission counts, and recent credit explanations. Read-only; reflects the local \
         view as of the last sync.",
        vec![EffectKind::ReadFilesystem],
        PermissionMode::Allow,
        resource_profile(),
    )
}

pub(super) fn profile_token_manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID,
        "Mint a short-lived Trace Commons profile-management value for the current user. \
         Prefer trace_commons.profile_set when the user wants to create or update \
         their public profile from the agent. Use this token only for browser/manual \
         profile setup. It is scoped only to community profile management; it cannot \
         submit traces. Pass confirmed=true only after the user has explicitly asked \
         to mint a manual/browser token in this conversation.",
        // Persists the minted token to a 0600 local file (out-of-band delivery,
        // keeping the bearer credential off the model surface), so the effect
        // model must declare the local filesystem write.
        vec![
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::Network,
            EffectKind::ExternalWrite,
        ],
        PermissionMode::Ask,
        Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(10_000)
                .set_output_bytes(FIRST_PARTY_DEFAULT_OUTPUT_BYTES),
            hard_ceiling: None,
        }),
    )
}

pub(super) fn profile_set_manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID,
        "Create or update the current user's public Trace Commons community profile. \
         ONLY call after the user has explicitly chosen a pseudonymous display handle \
         (and optional short bio) and confirmed they want it published. Pass \
         confirmed=true only when that consent was given in this conversation. \
         This is a separate public-profile opt-in and cannot submit traces.",
        vec![
            EffectKind::ReadFilesystem,
            EffectKind::Network,
            EffectKind::ExternalWrite,
        ],
        // Ask, not Allow: publishing a public community profile is an external
        // write to a public surface. The tool's `confirmed=true` input is
        // model-controlled (a prompt-injected model could supply it), so the
        // runtime approval gate — user-controlled consent — is the primary
        // control, with `confirmed=true` as defense-in-depth. profile_set is
        // also deliberately NOT on the local-dev approval-gate exemption list.
        PermissionMode::Ask,
        Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(15_000)
                .set_output_bytes(FIRST_PARTY_DEFAULT_OUTPUT_BYTES),
            hard_ceiling: None,
        }),
    )
}

pub(super) fn account_login_link_manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID,
        "Mint a one-time Trace Commons browser login link so the user can manage their \
         contributor account/profile in the web UI. Consent-gated: only call with \
         confirmed=true after the user explicitly asks. Routes through host network egress.",
        // ReadFilesystem: the dispatch reads local enrollment/policy/device-key
        // state before egress (mirrors profile_token's effect set).
        // WriteFilesystem: the minted one-time URL is persisted to a local
        // delivery file (never returned on the model-visible surface), so the
        // manifest must declare that credential-file write for policy/approval
        // surfaces.
        vec![
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::Network,
            EffectKind::ExternalWrite,
        ],
        PermissionMode::Ask,
        resource_profile(),
    )
}

// ── Input parsing ─────────────────────────────────────────────────────────────

struct OnboardToolInput {
    invite_url: String,
    consents: OnboardConsents,
    confirmed: bool,
}

#[derive(Debug)]
struct ProfileSetToolInput {
    display_handle: String,
    bio: Option<String>,
    confirmed: bool,
}

fn parse_onboard_input(input: &Value) -> Result<OnboardToolInput, FirstPartyCapabilityError> {
    let invite_url = input
        .get("invite_url")
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    if invite_url.is_empty() {
        return Err(input_error());
    }
    let include_message_text = input
        .get("include_message_text")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let include_tool_payloads = input
        .get("include_tool_payloads")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let confirmed = input
        .get("confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(OnboardToolInput {
        invite_url: invite_url.to_string(),
        consents: OnboardConsents {
            include_message_text,
            include_tool_payloads,
        },
        confirmed,
    })
}

fn parse_profile_set_input(
    input: &Value,
) -> Result<ProfileSetToolInput, FirstPartyCapabilityError> {
    let display_handle = input
        .get("display_handle")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|handle| !handle.is_empty())
        .ok_or_else(input_error)?;
    // Enforce the manifest's declared schema at parse time (don't rely on the
    // setter to bounce it deeper): handle is 3-32 ASCII letters/digits/`-`/`_`.
    // All-ASCII, so char count == byte length.
    if display_handle.len() < COMMUNITY_PROFILE_HANDLE_MIN_CHARS
        || display_handle.len() > COMMUNITY_PROFILE_HANDLE_MAX_CHARS
        || !display_handle
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(input_error());
    }
    let bio = input
        .get("bio")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|bio| !bio.is_empty())
        .map(str::to_string);
    // Bio is capped at the declared byte limit.
    if let Some(bio) = &bio
        && bio.len() > COMMUNITY_PROFILE_BIO_MAX_BYTES
    {
        return Err(input_error());
    }
    let confirmed = input
        .get("confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(ProfileSetToolInput {
        display_handle: display_handle.to_string(),
        bio,
        confirmed,
    })
}

// ── Host network-egress onboarding sink ───────────────────────────────────────

/// [`OnboardingHttpSink`] implementation that routes the onboarding POST through
/// the host runtime's network-egress policy, so the agent-invoked onboard tool
/// cannot reach private/internal destinations outside the deployment's outbound
/// allowlist. Mirrors `http::dispatch`'s `RuntimeHttpEgressRequest` construction.
struct HostEgressOnboardingSink {
    egress: Arc<dyn RuntimeHttpEgress>,
    scope: ResourceScope,
    capability_id: CapabilityId,
}

#[async_trait]
impl OnboardingHttpSink for HostEgressOnboardingSink {
    async fn post_onboard(
        &self,
        url: &str,
        body: Vec<u8>,
    ) -> Result<OnboardHttpResponse, OnboardError> {
        let request = RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: self.scope.clone(),
            capability_id: self.capability_id.clone(),
            method: NetworkMethod::Post,
            url: url.to_string(),
            headers: vec![
                ("accept".to_string(), "application/json".to_string()),
                ("content-type".to_string(), "application/json".to_string()),
            ],
            body,
            // First-party network policy is staged in HostHttpEgressService from
            // the grant obligation for this scope/capability; this request field
            // is the ignored fallback on that path (matches http::dispatch).
            network_policy: NetworkPolicy::default(),
            credential_injections: Vec::new(),
            response_body_limit: Some(ONBOARD_MAX_RESPONSE_BODY),
            // The onboarding response is parsed inline, never persisted to a
            // mount, so no save target is requested (matches http::dispatch's
            // `HttpSaveMode::Disabled` path).
            save_body_to: None,
            timeout_ms: Some(ONBOARD_TIMEOUT_MS),
        };
        let egress = self.egress.clone();
        // Catch a panic in the egress future so a faulty transport cannot abort
        // the onboarding task; map it to a sanitized network error.
        let response = AssertUnwindSafe(async move { egress.execute(request).await })
            .catch_unwind()
            .await
            .map_err(|_| {
                tracing::error!("trace_commons onboarding egress future panicked");
                OnboardError::Network {
                    reason: "onboarding egress worker failed".to_string(),
                }
            })?
            .map_err(map_egress_error)?;
        Ok(OnboardHttpResponse {
            status: response.status,
            body: response.body,
        })
    }
}

/// Map a host egress error to an `OnboardError`, without leaking
/// credential/secret detail into the reason string.
fn map_egress_error(error: RuntimeHttpEgressError) -> OnboardError {
    use ironclaw_host_api::RuntimeHttpEgressReasonCode as Code;
    let reason = error.stable_runtime_reason().to_string();
    match error.reason_code() {
        Code::CredentialUnavailable
        | Code::RequestDenied
        | Code::PolicyDenied
        | Code::NetworkError => OnboardError::Network { reason },
        Code::ResponseError | Code::ResponseBodyLimitExceeded => {
            OnboardError::MalformedResponse { reason }
        }
    }
}

// ── Host network-egress contribution sink ─────────────────────────────────────

/// [`ContributionHttpSink`] implementation that routes the AGENT-INVOKED
/// contribution writes (upload-claim mint, community-profile PUT/DELETE) through
/// the host runtime's network-egress policy, so the agent-invoked tools cannot
/// reach private/internal destinations outside the deployment's outbound
/// allowlist. Mirrors [`HostEgressOnboardingSink`].
struct HostEgressContributionSink {
    egress: Arc<dyn RuntimeHttpEgress>,
    scope: ResourceScope,
    capability_id: CapabilityId,
    /// One-shot stager used to deliver the host-minted Trace Commons bearer
    /// through the egress credential-injection path. `None` only on
    /// non-network-egress invocations, where a bearer would never be present.
    secret_stager: Option<RuntimeSecretMaterialStager>,
}

#[async_trait]
impl ContributionHttpSink for HostEgressContributionSink {
    async fn execute(
        &self,
        req: ContributionHttpRequest,
    ) -> Result<ContributionHttpResponse, ContributionHttpError> {
        let method = match req.method {
            ContributionHttpMethod::Get => NetworkMethod::Get,
            ContributionHttpMethod::Post => NetworkMethod::Post,
            ContributionHttpMethod::Put => NetworkMethod::Put,
            ContributionHttpMethod::Delete => NetworkMethod::Delete,
        };
        let headers = vec![
            ("accept".to_string(), "application/json".to_string()),
            ("content-type".to_string(), "application/json".to_string()),
        ];
        // The host-minted bearer is a credential: it MUST flow through the
        // staged credential-injection path, never as a raw Authorization
        // header. Writing it into `headers` here would (a) be denied by the
        // egress sensitive-header guard and (b) bypass the leased-secret
        // redaction the injection path provides. Stage the token one-shot,
        // then declare a `StagedObligation` injection targeting the
        // Authorization header; `apply_credential_injections` consumes it
        // after the guard runs.
        let mut credential_injections = Vec::new();
        if let Some(token) = req.bearer_token {
            let Some(secret_stager) = self.secret_stager.as_ref() else {
                return Err(ContributionHttpError::new(
                    "trace bearer staging is unavailable",
                ));
            };
            // Per-request unique handle: the injection store is a HashMap keyed
            // by (scope, capability, handle) with overwrite-on-insert, so a
            // constant handle would let two concurrent same-scope Trace Commons
            // egresses race — one staging over the other's bearer before it is
            // consumed. A uuid suffix makes every staged bearer key distinct.
            let handle_name = format!("{TRACE_COMMONS_BEARER_HANDLE}-{}", uuid::Uuid::new_v4());
            let handle = SecretHandle::new(&handle_name).map_err(|error| {
                // Safe to log the cause: the handle name is composed from a
                // compile-time constant plus a uuid, so this validation error
                // carries no secret/path — it only fires if that scheme is wrong.
                tracing::debug!(%error, "invalid trace bearer handle");
                ContributionHttpError::new("invalid trace bearer handle")
            })?;
            secret_stager
                .stage_secret_material_once(
                    &self.scope,
                    &self.capability_id,
                    &handle,
                    SecretMaterial::from(token),
                )
                .await
                .map_err(|_error| {
                    // This is the bearer-material path: the host-runtime logging
                    // guideline forbids emitting backend/storage error detail
                    // (`_error` may carry secret-store internals). Log only the
                    // safe fact of failure; the wire message stays sanitized.
                    tracing::debug!("trace bearer staging failed");
                    ContributionHttpError::new("trace bearer could not be staged")
                })?;
            credential_injections.push(RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: self.capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            });
        }
        let request = RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: self.scope.clone(),
            capability_id: self.capability_id.clone(),
            method,
            url: req.url,
            headers,
            body: req.json_body.unwrap_or_default(),
            // First-party network policy is staged in HostHttpEgressService from
            // the grant obligation for this scope/capability; this request field
            // is the ignored fallback on that path (matches http::dispatch).
            network_policy: NetworkPolicy::default(),
            credential_injections,
            response_body_limit: Some(req.response_body_limit),
            // The response is parsed inline, never persisted to a mount.
            save_body_to: None,
            timeout_ms: Some(req.timeout_ms),
        };
        let egress = self.egress.clone();
        // Catch a panic in the egress future so a faulty transport cannot abort
        // the contribution task; map it to a sanitized network error.
        let response = AssertUnwindSafe(async move { egress.execute(request).await })
            .catch_unwind()
            .await
            .map_err(|_| {
                tracing::error!("trace_commons contribution egress future panicked");
                ContributionHttpError::new("contribution egress worker failed")
            })?
            .map_err(map_egress_contribution_error)?;
        Ok(ContributionHttpResponse {
            status: response.status,
            body: response.body,
        })
    }
}

/// Map a host egress error to a `ContributionHttpError`, without leaking
/// credential/secret detail (or the URL/token) into the reason string. Reuses
/// the same stable-reason sanitization as [`map_egress_error`].
fn map_egress_contribution_error(error: RuntimeHttpEgressError) -> ContributionHttpError {
    ContributionHttpError::new(error.stable_runtime_reason())
}

// ── Onboard dispatch ──────────────────────────────────────────────────────────

pub(super) async fn dispatch_onboard(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let input = parse_onboard_input(&request.input)?;

    // Consent gate: never make a network call without explicit per-conversation
    // confirmation. This is a hard invariant — the model must gather consent
    // before calling with confirmed=true.
    if !input.confirmed {
        return Ok(json!({
            "enrolled": false,
            "consent_required": true,
            "message": "Before enrolling, confirm with the user that they want to \
        contribute redacted traces, and whether to include redacted message text and tool \
        payloads. Then call again with confirmed=true."
        }));
    }

    // The agent onboard path MUST route through host network egress — it must
    // never silently fall back to a direct client (mirrors http::dispatch).
    let egress = request
        .services
        .runtime_http_egress
        .as_ref()
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::NetworkDenied))?
        .clone();

    let scope = trace_scope_key(
        request.scope.tenant_id.as_str(),
        request.scope.user_id.as_str(),
    );
    let host_sink = HostEgressOnboardingSink {
        egress,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
    };
    match ironclaw_reborn_traces::onboarding::onboard(
        &scope,
        &input.invite_url,
        input.consents,
        &host_sink,
    )
    .await
    {
        Ok(outcome) => Ok(onboard_success_value(&outcome, &input.consents)),
        Err(e) => Ok(onboard_error_value(&e)),
    }
}

fn onboard_success_value(outcome: &OnboardOutcome, consents: &OnboardConsents) -> Value {
    let mut v = json!({
        "enrolled": true,
        "tenant_id": outcome.tenant_id,
        "ingest_url": outcome.ingest_url,
        "issuer_url": outcome.issuer_url,
        // device_key_id is the sha256 hash of the public key — safe to expose.
        "device_key_id": outcome.device_key_id,
        "consents": {
            "include_message_text": consents.include_message_text,
            "include_tool_payloads": consents.include_tool_payloads,
        },
        "next_steps": "Traces are redacted locally and queued; submission requires meeting \
    the score threshold. Optional second opt-in: to appear on the public community \
    leaderboard, choose a pseudonymous handle and ask the agent to set your public \
    Trace Commons profile, or run 'ironclaw-reborn traces profile set --handle \
    <pseudonymous-handle>'. Browser/manual profile setup can still use \
    'traces profile token'. \
    Opt out anytime with 'ironclaw traces opt-out'."
    });
    // Navigation hints are optional and only included when present (and HTTPS).
    if let Some(ref url) = outcome.community_url {
        v["community_url"] = Value::String(url.clone());
    }
    if let Some(ref url) = outcome.profile_url {
        v["profile_url"] = Value::String(url.clone());
    }
    if let Some(ref url) = outcome.leaderboard_url {
        v["leaderboard_url"] = Value::String(url.clone());
    }
    v
}

fn onboard_error_value(e: &OnboardError) -> Value {
    let (error_code, message) = match e {
        OnboardError::InviteRejected(OnboardErrorCode::InviteNotValid) => (
            "InviteRejected_InviteNotValid",
            "This invite link isn't valid — it may have been used already or revoked. \
Ask the operator for a new invite.",
        ),
        OnboardError::InviteRejected(OnboardErrorCode::OnboardRateLimited) => (
            "InviteRejected_OnboardRateLimited",
            "The server is rate-limiting onboarding attempts; try again in a few minutes.",
        ),
        OnboardError::InviteRejected(_) => (
            "InviteRejected",
            "The onboarding server rejected the invite.",
        ),
        OnboardError::InvalidInvite(_) => (
            "InvalidInvite",
            "That invite link is malformed. Double-check the link the operator gave you.",
        ),
        OnboardError::IssuerOriginMismatch { .. } => (
            "IssuerOriginMismatch",
            "The server response didn't match the invite link's origin; refusing to continue. \
The invite may be misconfigured — contact the operator.",
        ),
        OnboardError::InsecureIngestUrl { .. } => (
            "InsecureIngestUrl",
            "The server returned an insecure (non-HTTPS) endpoint; refusing to continue.",
        ),
        OnboardError::Network { .. } => (
            "Network",
            "Couldn't reach the onboarding server. The invite was not consumed; \
it's safe to retry.",
        ),
        OnboardError::MalformedResponse { .. } => (
            "MalformedResponse",
            "The onboarding server's response was malformed; contact the operator.",
        ),
        OnboardError::DeviceKey(_) => (
            "DeviceKeyError",
            "Couldn't establish the local device key for onboarding; the device-key state \
may be missing or malformed. Re-run onboarding with a fresh invite.",
        ),
        OnboardError::Persist { .. } => (
            "PersistError",
            "Couldn't save onboarding state locally; check disk and permissions, then retry.",
        ),
    };
    json!({
        "enrolled": false,
        "error_code": error_code,
        "message": message,
    })
}

// ── Status dispatch ───────────────────────────────────────────────────────────

pub(super) async fn dispatch_status(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    // Resolve the caller's effective enrollment — personal-invite OR the
    // admin-provisioned instance enrollment — so an instance-only contributor is
    // reported as enrolled (with the instance policy) rather than not-enrolled.
    // A MISSING policy is already softened to the not-enrolled default inside the
    // resolver's policy reads, so an `Err` here is a genuine read/parse failure
    // (unreadable or corrupt policy file). Do NOT mask that as `enrolled: false`
    // — a user who IS enrolled would be told they are not. Report the read
    // failure honestly without asserting an enrollment state.
    let resolution = match resolve_trace_credentials(
        &request.scope.tenant_id,
        &request.scope.user_id,
    ) {
        Ok(resolution) => resolution,
        Err(error) => {
            // The resolver error can embed the policy file's host path; the
            // host_runtime guideline forbids raw paths in logs. Log only the
            // safe fact, matching the sibling dispatchers.
            let _ = error;
            tracing::debug!("trace commons status: local policy read failed");
            return Ok(json!({
                "error_code": "PolicyReadFailed",
                "message": "Could not read local Trace Commons enrollment state; the policy file may be unreadable or corrupt."
            }));
        }
    };
    match resolution {
        Some(resolution) => Ok(format_status(&resolution.policy)),
        // Enrolled in neither personal nor instance: report the not-enrolled
        // default (enrolled: false), matching the prior missing-policy behavior.
        None => Ok(format_status(&StandingTraceContributionPolicy::default())),
    }
}

fn format_status(policy: &StandingTraceContributionPolicy) -> Value {
    let auth_mode = match policy.auth_mode {
        TraceUploadAuthMode::DeviceKey => "device_key",
        TraceUploadAuthMode::WorkloadTokenEnv => "workload_token_env",
    };
    json!({
        "enrolled": policy.enabled,
        "tenant_id": policy.upload_token_tenant_id,
        "auth_mode": auth_mode,
        "include_message_text": policy.include_message_text,
        "include_tool_payloads": policy.include_tool_payloads,
        "endpoint": policy.ingestion_endpoint,
    })
}

// ── Credits dispatch ──────────────────────────────────────────────────────────

pub(super) async fn dispatch_credits(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let scope = trace_scope_key(
        request.scope.tenant_id.as_str(),
        request.scope.user_id.as_str(),
    );
    // `scoped_credit_view` memoizes the full-history read/aggregate by the
    // on-disk input signature, so repeated calls on an unchanged history are
    // cheap. A MISSING submissions file is already softened to the zero state
    // inside the underlying readers, so an `Err` here is a genuine read/parse
    // failure (unreadable or corrupt records). Do NOT mask it as "no records" —
    // that would hide corruption/permission issues and under-report an active
    // contributor. Report the read failure honestly (mirrors `dispatch_status`).
    match ironclaw_reborn_traces::contribution::scoped_credit_view(scope.as_str()) {
        Ok(view) => Ok(format_credits(&view.report)),
        Err(error) => {
            tracing::debug!(%error, "trace commons credits: local records read failed");
            Ok(json!({
                "error_code": "RecordsReadFailed",
                "message": "Could not read local Trace Commons submission records; the records file may be unreadable or corrupt."
            }))
        }
    }
}

pub(crate) fn format_credits(report: &TraceCreditReport) -> Value {
    let last_submission_at = report
        .last_submission_at
        .map(|dt| Value::String(dt.to_rfc3339()))
        .unwrap_or(Value::Null);
    let last_credit_sync_at = report
        .last_credit_sync_at
        .map(|dt| Value::String(dt.to_rfc3339()))
        .unwrap_or(Value::Null);
    json!({
        "pending_credit": report.pending_credit,
        "final_credit": report.final_credit,
        "delayed_credit_delta": report.delayed_credit_delta,
        "submissions_total": report.submissions_total,
        "submissions_submitted": report.submissions_submitted,
        "submissions_accepted": report.submissions_accepted,
        "submissions_revoked": report.submissions_revoked,
        "submissions_expired": report.submissions_expired,
        "credit_events_total": report.credit_events_total,
        "last_submission_at": last_submission_at,
        "last_credit_sync_at": last_credit_sync_at,
        "recent_explanations": report.explanation_lines,
        "note": "Local view as of last sync; final credit can change after privacy review, \
    replay/eval, duplicate checks, and downstream utility scoring. \
    The authoritative ledger is server-side."
    })
}

// ── Profile token dispatch ───────────────────────────────────────────────────

pub(super) async fn dispatch_profile_token(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    // Consent gate: minting persists a bearer profile-management credential.
    // The runtime approval gate (PermissionMode::Ask) can be auto-approved in
    // local-yolo, so this in-turn confirmed=true check is the hard fail-closed
    // boundary — mirroring dispatch_onboard / dispatch_profile_set. Never mint a
    // credential without explicit per-conversation confirmation.
    let confirmed = request
        .input
        .get("confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !confirmed {
        return Ok(json!({
            "minted": false,
            "consent_required": true,
            "message": "Minting a Trace Commons profile-management token persists a bearer \
        credential for manual/browser setup. Prefer trace_commons.profile_set to set the public \
        profile directly. If the user explicitly wants a manual token, confirm with them first, \
        then call again with confirmed=true."
        }));
    }

    let scope = trace_scope_key(
        request.scope.tenant_id.as_str(),
        request.scope.user_id.as_str(),
    );

    // Enrollment pre-check BEFORE extracting host egress: a not-enrolled user
    // must get NotEnrolled guidance, not a NetworkDenied miswiring error. This
    // mirrors dispatch_profile_set's ordering (enrollment check precedes the
    // network call). Egress extraction below is the host-runtime miswiring
    // guard for the confirmed+enrolled mint path. Route through the shared
    // resolver so instance-only contributors (personal policy absent, instance
    // policy enabled) pass the gate instead of being falsely rejected.
    match resolve_trace_credentials(&request.scope.tenant_id, &request.scope.user_id) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Ok(profile_token_error_value(
                &ProfileAttributionError::NotEnrolled,
            ));
        }
        Err(error) => {
            return Ok(profile_token_error_value(
                &ProfileAttributionError::PolicyRead(error),
            ));
        }
    }

    // The agent profile_token path MUST route through host network egress — it
    // must never silently fall back to a direct client (mirrors dispatch_onboard).
    let egress = match request.services.runtime_http_egress.as_ref() {
        Some(egress) => egress.clone(),
        None => {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::NetworkDenied,
            ));
        }
    };
    let sink = HostEgressContributionSink {
        egress,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
        secret_stager: request.services.runtime_secret_material_stager.clone(),
    };
    match mint_profile_attribution_token_for_user_via_sink(
        &request.scope.tenant_id,
        &request.scope.user_id,
        &sink,
    )
    .await
    {
        Ok(token) => match persist_profile_token(&scope, &token) {
            Ok(_path) => Ok(format_profile_token(&token)),
            Err(error) => {
                // Filesystem write error can carry host paths; log only the fact.
                let _ = error;
                tracing::debug!("failed to persist Trace Commons profile token");
                Ok(profile_token_error_value(
                    &ProfileAttributionError::LocalStateWrite,
                ))
            }
        },
        Err(error) => Ok(profile_token_error_value(&error)),
    }
}

/// Write the raw bearer token to a 0600 file in the scope's local state dir and
/// return its path. The token is a credential: it must NOT be returned in the
/// model-visible tool result (that copies it into the LLM transcript/history
/// and any downstream persistence). Delivering it out-of-band via a private
/// file keeps the secret off the model surface while the manual browser-setup
/// flow can still read it.
fn persist_profile_token(scope: &str, token: &ProfileAttributionToken) -> std::io::Result<PathBuf> {
    use std::io::Write as _;

    let dir = trace_contribution_dir_for_scope(Some(scope));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("profile_token.jwt");

    // Write atomically: a unique 0600 temp file (per-write uuid name so
    // concurrent mints don't race on a fixed temp path), fsync, then rename
    // onto the final path. A reader of `profile_token.jwt` therefore only ever
    // sees a complete token, never a half-written or overwritten credential —
    // the same temp+rename discipline the codebase uses for other credential
    // writes.
    let temp_path = dir.join(format!("profile_token.jwt.{}.tmp", uuid::Uuid::new_v4()));
    let write_temp = || -> std::io::Result<()> {
        #[cfg(unix)]
        let mut file = {
            use std::os::unix::fs::OpenOptionsExt as _;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp_path)?
        };
        #[cfg(not(unix))]
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(token.access_token.as_bytes())?;
        file.sync_all()
    };
    let cleanup_temp = |context: &'static str| {
        if let Err(cleanup_error) = std::fs::remove_file(&temp_path) {
            tracing::debug!(
                error = ?cleanup_error,
                context,
                "best-effort cleanup of profile token temp file failed"
            );
        }
    };
    if let Err(error) = write_temp() {
        cleanup_temp("write");
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temp_path, &path) {
        cleanup_temp("rename");
        return Err(error);
    }
    Ok(path)
}

fn format_profile_token(token: &ProfileAttributionToken) -> Value {
    json!({
        "minted": true,
        "token_type": "Bearer",
        // The raw access_token is deliberately NOT included here — it is a
        // bearer credential and must not enter the model transcript. Neither is
        // the host path of the file it was written to: an absolute/local path
        // is a host detail that must not cross a model/user-visible surface.
        // The token is persisted (0600) for out-of-band retrieval by a
        // bearer-auth UI/CLI; `token_delivery` is an opaque marker for that.
        "token_delivery": "local_private_profile_token_file",
        "expires_at": token.expires_at.as_ref().map(|dt| dt.to_rfc3339()),
        "expires_in": token.expires_in,
        "consent_scope": "public_attribution",
        "allowed_uses": [],
        // No profile URL is surfaced here: the token is a bearer credential
        // scoped to the user's ENROLLED issuer (which may be self-hosted or
        // loopback). Naming a fixed origin would risk steering the user to
        // paste the token at the wrong host. The enrolled profile flow / local
        // UI/CLI knows the correct origin out of band.
        "message": "Prefer asking the agent to set your public profile directly with a pseudonymous handle. \
    For browser/manual setup only, use the local Trace Commons UI or CLI to open the private profile-token \
    file, then continue through your enrolled Trace Commons profile flow (do not paste it at any other origin). \
    The token is not shown here because it is a credential and must not appear in the conversation."
    })
}

fn profile_token_error_value(error: &ProfileAttributionError) -> Value {
    // Typed variants → public `error_code`; no substring matching on wording.
    let (error_code, message) = match error {
        ProfileAttributionError::NotEnrolled => (
            "NotEnrolled",
            "Trace Commons enrollment was not found for this user. Onboard with the operator invite link first.",
        ),
        ProfileAttributionError::PolicyRead(_) => (
            "PolicyReadFailed",
            "Could not read local Trace Commons enrollment state; the policy file may be unreadable or corrupt.",
        ),
        ProfileAttributionError::EnrollmentIncomplete(_) => (
            "EnrollmentIncomplete",
            "Trace Commons enrollment is incomplete (missing upload-claim issuer URL or device-key state). Re-run onboarding with a fresh invite.",
        ),
        ProfileAttributionError::Backend(_) => (
            "ProfileTokenMintFailed",
            "Could not mint a Trace Commons profile token. Check enrollment status and retry.",
        ),
        ProfileAttributionError::LocalStateWrite => (
            "LocalStateWriteFailed",
            "Could not write the profile token to local state.",
        ),
    };
    json!({
        "minted": false,
        "error_code": error_code,
        "message": message,
    })
}

// ── Profile set dispatch ─────────────────────────────────────────────────────

pub(super) async fn dispatch_profile_set(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let input = parse_profile_set_input(&request.input)?;

    // Consent gate: publishing/updating the public community profile is a
    // separate public-attribution opt-in. The runtime approval gate is the
    // primary user-controlled consent boundary (profile_set is NOT exempt and
    // is registered PermissionMode::Ask); this input check is defense-in-depth.
    // Never reach the network without explicit per-conversation confirmation.
    if !input.confirmed {
        return Ok(json!({
            "updated": false,
            "consent_required": true,
            "message": "Publishing or updating the public Trace Commons community profile is a \
        separate public-attribution opt-in. Confirm the display handle (and optional bio) with \
        the user first, then call again with confirmed=true."
        }));
    }

    // Route the enrollment gate through the shared resolver so instance-only
    // contributors (personal policy absent, instance policy enabled) pass
    // instead of being falsely rejected as not enrolled.
    match resolve_trace_credentials(&request.scope.tenant_id, &request.scope.user_id) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Ok(profile_set_error_value(
                &CommunityProfileError::Attribution(ProfileAttributionError::NotEnrolled),
            ));
        }
        Err(error) => {
            return Ok(profile_set_error_value(
                &CommunityProfileError::Attribution(ProfileAttributionError::PolicyRead(error)),
            ));
        }
    }

    // The agent profile_set path MUST route through host network egress — it
    // must never silently fall back to a direct client (mirrors dispatch_onboard).
    // Extracted AFTER the enrollment check so a not-enrolled user gets NotEnrolled
    // guidance rather than a NetworkDenied miswiring error.
    let egress = match request.services.runtime_http_egress.as_ref() {
        Some(egress) => egress.clone(),
        None => {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::NetworkDenied,
            ));
        }
    };
    let sink = HostEgressContributionSink {
        egress,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
        secret_stager: request.services.runtime_secret_material_stager.clone(),
    };
    match set_community_profile_for_user_via_sink(
        &request.scope.tenant_id,
        &request.scope.user_id,
        &input.display_handle,
        input.bio.as_deref(),
        &sink,
    )
    .await
    {
        Ok(()) => Ok(profile_set_success_value(&input)),
        Err(error) => Ok(profile_set_error_value(&error)),
    }
}

fn profile_set_success_value(input: &ProfileSetToolInput) -> Value {
    json!({
        "updated": true,
        "display_handle": input.display_handle.as_str(),
        "bio": input.bio.as_deref(),
        // No fixed profile URL: the profile was published to the user's enrolled
        // issuer (possibly self-hosted/loopback), so naming tracecommons.ai here
        // would misdirect a self-hosted user.
        "message": "Your public Trace Commons profile handle is set. It may appear on the leaderboard after the next snapshot.",
    })
}

fn profile_set_error_value(error: &CommunityProfileError) -> Value {
    // Typed variants → public `error_code`; no substring matching on wording.
    let (error_code, message) = match error {
        CommunityProfileError::InvalidProfile(_) => (
            "InvalidProfile",
            "Choose a pseudonymous handle 3-32 characters long using ASCII letters, digits, '-' or '_'. Bio must be at most 280 bytes.",
        ),
        CommunityProfileError::Attribution(ProfileAttributionError::NotEnrolled) => (
            "NotEnrolled",
            "Trace Commons enrollment was not found for this user. Onboard with the operator invite link first.",
        ),
        CommunityProfileError::Attribution(ProfileAttributionError::PolicyRead(_)) => (
            "PolicyReadFailed",
            "Could not read local Trace Commons enrollment state; the policy file may be unreadable or corrupt.",
        ),
        CommunityProfileError::Attribution(ProfileAttributionError::EnrollmentIncomplete(_)) => (
            "EnrollmentIncomplete",
            "Trace Commons enrollment is incomplete (missing upload-claim issuer URL or device-key state). Re-run onboarding with a fresh invite.",
        ),
        // profile_set does not persist local state; `LocalStateWrite` cannot
        // occur here, but the match stays exhaustive over the shared enum.
        CommunityProfileError::Attribution(
            ProfileAttributionError::Backend(_) | ProfileAttributionError::LocalStateWrite,
        ) => (
            "ProfileSetFailed",
            "Could not update the Trace Commons public profile. Check enrollment status and retry.",
        ),
    };
    json!({
        "updated": false,
        "error_code": error_code,
        "message": message,
    })
}

// ── Account login link dispatch ───────────────────────────────────────────────

pub(super) async fn dispatch_account_login_link(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    // Consent gate: minting a one-time login link is an account-access action.
    // The runtime approval gate (PermissionMode::Ask) can be auto-approved in
    // local-yolo, so this in-band confirmed=true check is the hard fail-closed
    // boundary — mirroring dispatch_profile_token / dispatch_onboard. Never
    // mint a login link without explicit per-conversation confirmation.
    let confirmed = request
        .input
        .get("confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !confirmed {
        return Ok(json!({
            "minted": false,
            "consent_required": true,
            "message": "Minting a Trace Commons browser login link opens account access for \
        the user. Confirm with the user that they explicitly want to log in to their Trace \
        Commons account, then call again with confirmed=true."
        }));
    }

    let scope = trace_scope_key(
        request.scope.tenant_id.as_str(),
        request.scope.user_id.as_str(),
    );

    // Enrollment pre-check BEFORE extracting host egress: a not-enrolled user
    // must get NotEnrolled guidance, not a NetworkDenied miswiring error.
    // Mirrors dispatch_profile_token's ordering. Route through the shared
    // resolver so instance-only contributors pass the gate — matching
    // `mint_account_login_link_via_sink`, which already resolves instance
    // enrollment via `resolve_trace_credentials`.
    match resolve_trace_credentials(&request.scope.tenant_id, &request.scope.user_id) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Ok(account_login_link_error_value(
                &AccountLoginLinkError::NotEnrolled,
            ));
        }
        Err(error) => {
            return Ok(account_login_link_error_value(
                &AccountLoginLinkError::PolicyRead(error),
            ));
        }
    }

    // The agent account_login_link path MUST route through host network egress
    // — it must never silently fall back to a direct client (mirrors
    // dispatch_onboard / dispatch_profile_token).
    let egress = match request.services.runtime_http_egress.as_ref() {
        Some(egress) => egress.clone(),
        None => {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::NetworkDenied,
            ));
        }
    };
    let sink = HostEgressContributionSink {
        egress,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
        secret_stager: request.services.runtime_secret_material_stager.clone(),
    };
    match mint_account_login_link_via_sink(&request.scope.tenant_id, &request.scope.user_id, &sink)
        .await
    {
        Ok(link) => {
            // The persist path is blocking fs (mkdir/write/fsync/rename); keep it
            // off the async worker via spawn_blocking so we never stall a Tokio
            // thread on disk I/O.
            let scope_owned = scope.clone();
            let link_for_persist = link.clone();
            let persisted = tokio::task::spawn_blocking(move || {
                persist_account_login_link(&scope_owned, &link_for_persist)
            })
            .await;
            match persisted {
                Ok(Ok(_)) => Ok(format_account_login_link(&link)),
                Ok(Err(error)) => {
                    // Filesystem errors can carry raw host paths (mkdir/write/
                    // fsync/rename); the host_runtime guideline forbids that in
                    // logs. Log only the safe fact; message stays sanitized.
                    let _ = error;
                    tracing::debug!("failed to persist Trace Commons account login link");
                    Ok(account_login_link_error_value(
                        &AccountLoginLinkError::LocalStateWrite,
                    ))
                }
                Err(join_error) => {
                    let _ = join_error;
                    tracing::debug!("account login link persist task failed");
                    Ok(account_login_link_error_value(
                        &AccountLoginLinkError::LocalStateWrite,
                    ))
                }
            }
        }
        Err(error) => Ok(account_login_link_error_value(&error)),
    }
}

/// Write the one-time login URL to a private local file in the scope's local
/// state dir (created with mode `0600` on Unix; default inherited permissions
/// elsewhere) and return its path. The URL is a code-bearing account-access
/// credential: it must NOT be returned in the model-visible tool result (that
/// copies it into the LLM transcript/history and any downstream persistence).
/// Delivering it out-of-band via a private file keeps the secret off the model
/// surface while the local browser-login flow can still read it. Mirrors
/// `persist_profile_token`.
fn persist_account_login_link(scope: &str, link: &AccountLoginLink) -> std::io::Result<PathBuf> {
    use std::io::Write as _;

    let dir = trace_contribution_dir_for_scope(Some(scope));
    std::fs::create_dir_all(&dir)?;
    // Each mint gets its own final file (uuid suffix): two concurrent same-scope
    // mints both return minted=true, so a shared fixed path would let the later
    // rename silently clobber the earlier caller's only copy of its one-time
    // URL. Readers list `account_login_link.*.url` files rather than assuming a
    // fixed name. Stale siblings are pruned best-effort below (the links
    // themselves are one-time-use and expire server-side).
    let mint_id = uuid::Uuid::new_v4();
    let path = dir.join(format!("account_login_link.{mint_id}.url"));
    prune_stale_account_login_links(&dir);

    // Atomic write: a unique temp file (per-write uuid name so concurrent mints
    // don't race on a fixed temp path; created 0600 on Unix), fsync, then rename
    // onto the final path — the same temp+rename credential-write discipline as
    // the profile token, so a reader only ever sees a complete URL.
    let temp_path = dir.join(format!("account_login_link.{mint_id}.tmp"));
    let write_temp = || -> std::io::Result<()> {
        #[cfg(unix)]
        let mut file = {
            use std::os::unix::fs::OpenOptionsExt as _;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp_path)?
        };
        #[cfg(not(unix))]
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(link.url.as_bytes())?;
        file.sync_all()
    };
    if let Err(error) = write_temp() {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temp_path, &path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(path)
}

/// Best-effort removal of expired login-link delivery files (and orphaned temp
/// files) older than one hour. The minted links are one-time-use and expire
/// server-side well within that window, so anything older is dead credential
/// material that should not accumulate on disk. Errors are ignored: pruning
/// must never fail a fresh mint.
fn prune_stale_account_login_links(dir: &std::path::Path) {
    const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60 * 60);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let is_link_file = name.starts_with("account_login_link.")
            && (name.ends_with(".url") || name.ends_with(".tmp"));
        if !is_link_file {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if now.duration_since(modified).is_ok_and(|age| age > MAX_AGE) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn format_account_login_link(link: &AccountLoginLink) -> Value {
    json!({
        "minted": true,
        "account_id": link.account_id,
        // The one-time login URL is deliberately NOT included here — it is a
        // code-bearing account-access credential and must not enter the model
        // transcript. It is persisted as a private local file (0600 on Unix) for
        // out-of-band retrieval by the local Trace Commons UI/CLI; `link_delivery`
        // is an opaque marker (never a host path, which is itself a host detail
        // that must not cross the
        // model/user-visible surface).
        "link_delivery": "local_private_account_login_link_file",
        "message": "A one-time Trace Commons browser login link was minted but is not shown here \
    because it is an account-access credential and must not appear in the conversation. Use the \
    local Trace Commons UI or CLI to open the private account-login-link file in your browser. \
    It is one-time-use and expires shortly.",
    })
}

fn account_login_link_error_value(error: &AccountLoginLinkError) -> Value {
    // The public `error_code` contract is derived from typed variants, not from
    // substring-matching upstream error wording.
    let (error_code, message) = match error {
        AccountLoginLinkError::NotEnrolled => (
            "NotEnrolled",
            "Trace Commons enrollment was not found for this user. Onboard with the operator invite link first.",
        ),
        AccountLoginLinkError::PolicyRead(_) => (
            "PolicyReadFailed",
            "Could not read local Trace Commons enrollment state; the policy file may be unreadable or corrupt.",
        ),
        AccountLoginLinkError::EnrollmentIncomplete(_) => (
            "EnrollmentIncomplete",
            "Trace Commons enrollment is incomplete (missing upload-claim issuer URL or device-key state). Re-run onboarding with a fresh invite.",
        ),
        AccountLoginLinkError::IssuerRefused { .. } => (
            "IssuerRefused",
            "The Trace Commons issuer refused to mint a login link. Ask the operator to check account/device-key status.",
        ),
        AccountLoginLinkError::Backend(_) => (
            "AccountLoginLinkFailed",
            "Could not mint a Trace Commons account login link. Check enrollment status and retry.",
        ),
        AccountLoginLinkError::LocalStateWrite => (
            "LocalStateWriteFailed",
            "Could not write the account login link to local state.",
        ),
    };
    json!({
        "minted": false,
        "error_code": error_code,
        "message": message,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use ironclaw_filesystem::DiskFilesystem;
    use ironclaw_host_api::{CapabilityId, ResourceEstimate, ResourceScope};
    use ironclaw_reborn_traces::contribution::{
        StandingTraceContributionPolicy, TraceCreditReport,
    };
    use serde_json::json;

    use crate::{
        CommandExecutionOutput, CommandExecutionRequest, InvocationServices, RuntimeProcessError,
        RuntimeProcessPort,
    };

    use super::*;

    struct NoopProcessPort;

    #[async_trait]
    impl RuntimeProcessPort for NoopProcessPort {
        async fn run_command(
            &self,
            _request: CommandExecutionRequest,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            unreachable!("trace_commons tests must not execute commands")
        }
    }

    /// Build a minimal `FirstPartyCapabilityRequest` for unit tests.
    /// Uses the system scope so no validated user/tenant id is needed.
    fn test_request(input: Value) -> FirstPartyCapabilityRequest {
        FirstPartyCapabilityRequest {
            run_id: None,
            capability_id: CapabilityId::new(TRACE_COMMONS_ONBOARD_CAPABILITY_ID).unwrap(),
            scope: ResourceScope::system(),
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default(),
            mounts: None,
            services: InvocationServices {
                filesystem: Arc::new(DiskFilesystem::new()),
                runtime_http_egress: None,
                tool_call_http_egress: None,
                runtime_secret_material_stager: None,
                process: Arc::new(NoopProcessPort),
                secret_store: None,
                audit_sink: None,
                unsafe_raw_diagnostics_allowed: false,
                post_edit_check: None,
            },
            input,
        }
    }

    // ── parse_onboard_input tests ─────────────────────────────────────────────

    #[test]
    fn parse_onboard_input_rejects_missing_invite_url() {
        let err = parse_onboard_input(&json!({}));
        assert!(err.is_err(), "missing invite_url must be rejected");
    }

    #[test]
    fn parse_onboard_input_rejects_empty_invite_url() {
        let err = parse_onboard_input(&json!({ "invite_url": "" }));
        assert!(err.is_err(), "empty invite_url must be rejected");
    }

    #[test]
    fn parse_onboard_input_parses_confirmed_and_consents() {
        let input = parse_onboard_input(&json!({
            "invite_url": "https://tc.example.com/onboard#CODE1",
            "include_message_text": true,
            "include_tool_payloads": false,
            "confirmed": true,
        }))
        .unwrap();
        assert_eq!(input.invite_url, "https://tc.example.com/onboard#CODE1");
        assert!(input.consents.include_message_text);
        assert!(!input.consents.include_tool_payloads);
        assert!(input.confirmed);
    }

    #[test]
    fn parse_onboard_input_defaults_confirmed_and_consents_to_false() {
        let input =
            parse_onboard_input(&json!({ "invite_url": "https://tc.example.com/onboard#X" }))
                .unwrap();
        assert!(!input.confirmed);
        assert!(!input.consents.include_message_text);
        assert!(!input.consents.include_tool_payloads);
    }

    // ── Consent gate test ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn dispatch_onboard_without_confirmed_returns_consent_required_no_network() {
        // confirmed=false short-circuits before any network call — deterministic.
        let request = test_request(json!({
            "invite_url": "https://tc.example.com/onboard#TESTCODE",
            "confirmed": false,
        }));
        let result = dispatch_onboard(&request).await.unwrap();
        assert_eq!(result["enrolled"], json!(false));
        assert_eq!(result["consent_required"], json!(true));
        assert!(result["message"].as_str().is_some_and(|m| !m.is_empty()));
    }

    #[tokio::test]
    async fn dispatch_onboard_confirmed_without_host_egress_is_network_denied() {
        // The agent onboard path MUST route through host network egress and must
        // never fall back to a direct client. `test_request` wires
        // `runtime_http_egress: None`, so a confirmed onboard fails closed with
        // NetworkDenied (host-runtime miswiring guard).
        let request = test_request(json!({
            "invite_url": "https://tc.example.com/onboard#TESTCODE",
            "confirmed": true,
        }));
        let error = dispatch_onboard(&request)
            .await
            .expect_err("confirmed onboard without host egress must fail closed");
        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::NetworkDenied));
    }

    #[test]
    fn parse_profile_set_input_enforces_handle_and_bio_schema_limits() {
        // Too short, too long, and disallowed characters are rejected.
        assert!(parse_profile_set_input(&json!({"display_handle": "ab"})).is_err());
        assert!(parse_profile_set_input(&json!({"display_handle": "a".repeat(33)})).is_err());
        assert!(parse_profile_set_input(&json!({"display_handle": "has space"})).is_err());
        assert!(parse_profile_set_input(&json!({"display_handle": "emoji😀"})).is_err());
        // Bio over the declared byte cap is rejected.
        assert!(
            parse_profile_set_input(&json!({
                "display_handle": "pilot_zaki",
                "bio": "x".repeat(COMMUNITY_PROFILE_BIO_MAX_BYTES + 1)
            }))
            .is_err()
        );
        // A handle at each boundary + a max-length bio is accepted.
        assert!(parse_profile_set_input(&json!({"display_handle": "abc"})).is_ok());
        assert!(parse_profile_set_input(&json!({"display_handle": "a".repeat(32)})).is_ok());
        assert!(
            parse_profile_set_input(&json!({
                "display_handle": "pilot-zaki_1",
                "bio": "x".repeat(COMMUNITY_PROFILE_BIO_MAX_BYTES)
            }))
            .is_ok()
        );
    }

    // ── onboard_success_value tests ───────────────────────────────────────────

    #[test]
    fn onboard_success_value_includes_required_fields_and_no_key_material() {
        let outcome = OnboardOutcome {
            tenant_id: "tenant-abc".to_string(),
            ingest_url: "https://ingest.example.com".to_string(),
            issuer_url: "https://issuer.example.com".to_string(),
            device_key_id: "sha256:abcdef".to_string(),
            contributor_label: None,
            community_url: None,
            profile_url: None,
            leaderboard_url: None,
        };
        let consents = OnboardConsents {
            include_message_text: true,
            include_tool_payloads: false,
        };
        let v = onboard_success_value(&outcome, &consents);
        assert_eq!(v["enrolled"], json!(true));
        assert_eq!(v["tenant_id"], json!("tenant-abc"));
        assert_eq!(v["device_key_id"], json!("sha256:abcdef"));

        // No private key material.
        let serialized = serde_json::to_string(&v).unwrap();
        assert!(
            !serialized.contains("private"),
            "output must not contain 'private'"
        );
        assert!(
            !serialized.contains("BEGIN"),
            "output must not contain 'BEGIN' (no PEM key material)"
        );

        // Community URLs absent when None.
        assert!(v.get("community_url").is_none());
        assert!(v.get("profile_url").is_none());
        assert!(v.get("leaderboard_url").is_none());
    }

    #[test]
    fn onboard_success_value_includes_community_urls_when_some() {
        let outcome = OnboardOutcome {
            tenant_id: "tenant-x".to_string(),
            ingest_url: "https://ingest.example.com".to_string(),
            issuer_url: "https://issuer.example.com".to_string(),
            device_key_id: "sha256:ff".to_string(),
            contributor_label: None,
            community_url: Some("https://tracecommons.ai".to_string()),
            profile_url: Some("https://tracecommons.ai/profile".to_string()),
            leaderboard_url: Some("https://tracecommons.ai/lb".to_string()),
        };
        let consents = OnboardConsents::default();
        let v = onboard_success_value(&outcome, &consents);
        assert_eq!(v["community_url"], json!("https://tracecommons.ai"));
        assert_eq!(v["profile_url"], json!("https://tracecommons.ai/profile"));
        assert_eq!(v["leaderboard_url"], json!("https://tracecommons.ai/lb"));
    }

    // ── onboard_error_value tests ─────────────────────────────────────────────

    #[test]
    fn onboard_error_value_maps_invite_not_valid() {
        let e = OnboardError::InviteRejected(OnboardErrorCode::InviteNotValid);
        let v = onboard_error_value(&e);
        assert_eq!(v["enrolled"], json!(false));
        assert_eq!(v["error_code"], json!("InviteRejected_InviteNotValid"));
        let msg = v["message"].as_str().unwrap();
        assert!(!msg.is_empty(), "message must be non-empty");
    }

    #[test]
    fn onboard_error_value_maps_network_error() {
        let e = OnboardError::Network {
            reason: "connection refused".to_string(),
        };
        let v = onboard_error_value(&e);
        assert_eq!(v["enrolled"], json!(false));
        assert_eq!(v["error_code"], json!("Network"));
        let msg = v["message"].as_str().unwrap();
        assert!(!msg.is_empty(), "message must be non-empty");
        // Must not leak the raw network error reason.
        assert!(
            !msg.contains("connection refused"),
            "message must not leak internal error detail"
        );
    }

    // ── format_status tests ───────────────────────────────────────────────────

    #[test]
    fn format_status_enabled_device_key_policy() {
        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_auth_mode(TraceUploadAuthMode::DeviceKey)
            .set_upload_token_tenant_id("tenant-z")
            .set_include_message_text(true)
            .set_include_tool_payloads(false)
            .set_ingestion_endpoint("https://ingest.example.com");
        let v = format_status(&policy);
        assert_eq!(v["enrolled"], json!(true));
        assert_eq!(v["auth_mode"], json!("device_key"));
        assert_eq!(v["tenant_id"], json!("tenant-z"));
        assert_eq!(v["include_message_text"], json!(true));
    }

    #[test]
    fn format_status_default_disabled_policy() {
        let policy = StandingTraceContributionPolicy::default();
        let v = format_status(&policy);
        assert_eq!(v["enrolled"], json!(false));
        assert_eq!(v["auth_mode"], json!("workload_token_env"));
    }

    // ── format_credits tests ──────────────────────────────────────────────────

    #[test]
    fn format_credits_reports_balances() {
        let fixed_dt: DateTime<Utc> = DateTime::parse_from_rfc3339("2025-01-15T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let report = TraceCreditReport {
            submissions_total: 5,
            submissions_submitted: 3,
            submissions_revoked: 0,
            submissions_expired: 1,
            submissions_accepted: 2,
            submissions_quarantined: 0,
            submissions_rejected: 0,
            pending_credit: 1.5,
            final_credit: 0.25,
            credit_events_total: 4,
            delayed_credit_delta: 0.0,
            last_submission_at: Some(fixed_dt),
            last_credit_sync_at: None,
            explanation_lines: vec!["+0.10 regression catch".to_string()],
        };
        let v = format_credits(&report);
        assert_eq!(v["pending_credit"], json!(1.5_f32));
        assert_eq!(v["final_credit"], json!(0.25_f32));
        assert_eq!(v["submissions_submitted"], json!(3_u32));
        assert_eq!(v["recent_explanations"], json!(["+0.10 regression catch"]));
        assert_eq!(v["last_submission_at"], json!("2025-01-15T10:00:00+00:00"));
        assert_eq!(v["last_credit_sync_at"], json!(null));
        let note = v["note"].as_str().unwrap();
        assert!(
            note.contains("authoritative ledger is server-side"),
            "note must reference the authoritative server-side ledger"
        );
    }

    #[test]
    fn format_credits_empty_report() {
        let report = TraceCreditReport {
            submissions_total: 0,
            submissions_submitted: 0,
            submissions_revoked: 0,
            submissions_expired: 0,
            submissions_accepted: 0,
            submissions_quarantined: 0,
            submissions_rejected: 0,
            pending_credit: 0.0,
            final_credit: 0.0,
            credit_events_total: 0,
            delayed_credit_delta: 0.0,
            last_submission_at: None,
            last_credit_sync_at: None,
            explanation_lines: vec![],
        };
        let v = format_credits(&report);
        assert_eq!(v["pending_credit"], json!(0.0_f32));
        assert_eq!(v["submissions_total"], json!(0_u32));
        assert_eq!(v["recent_explanations"], json!([]));
        assert_eq!(v["last_submission_at"], json!(null));
        assert_eq!(v["last_credit_sync_at"], json!(null));
    }

    #[test]
    fn format_profile_token_never_exposes_raw_token_on_model_surface() {
        let expires_at: DateTime<Utc> = DateTime::parse_from_rfc3339("2026-06-09T20:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let token = ProfileAttributionToken {
            access_token: "eyJ.profile.token".to_string(),
            expires_at: Some(expires_at),
            expires_in: Some(300),
        };
        let v = format_profile_token(&token);
        assert_eq!(v["minted"], json!(true));
        assert_eq!(v["token_type"], json!("Bearer"));
        // The raw bearer credential must NOT appear in the model-visible result
        // (it is a secret; the model transcript would otherwise persist it).
        assert!(
            v.get("access_token").is_none(),
            "access_token must not be in the model-visible result"
        );
        assert!(
            !serde_json::to_string(&v)
                .unwrap()
                .contains("eyJ.profile.token"),
            "raw token must not appear anywhere in the serialized output"
        );
        // The token is delivered out-of-band via an opaque marker — never a
        // host filesystem path (an absolute/local path is a host detail that
        // must not cross a model/user-visible surface).
        assert_eq!(
            v["token_delivery"],
            json!("local_private_profile_token_file"),
            "out-of-band delivery is signaled by an opaque marker, not a path"
        );
        assert!(
            v.get("token_file").is_none(),
            "no host path field may be present on the model-visible result"
        );
        assert_eq!(v["consent_scope"], json!("public_attribution"));
        let message = v["message"].as_str().unwrap();
        assert!(
            message.contains("agent to set your public profile directly"),
            "message must prefer direct agent profile setup"
        );
        assert!(
            message.contains("local Trace Commons UI or CLI"),
            "message must point the user at the out-of-band retrieval path"
        );
    }

    #[test]
    fn profile_token_error_maps_missing_enrollment_without_raw_error() {
        assert_eq!(
            profile_token_error_value(&ProfileAttributionError::NotEnrolled)["error_code"],
            json!("NotEnrolled")
        );
        // The source cause is carried in the variant but the mapper emits only a
        // static message — the raw error text never reaches the model surface.
        let v = profile_token_error_value(&ProfileAttributionError::PolicyRead(
            std::io::Error::other("secret path /home/x - onboard first").into(),
        ));
        assert_eq!(v["minted"], json!(false));
        assert_eq!(v["error_code"], json!("PolicyReadFailed"));
        assert!(
            !serde_json::to_string(&v).unwrap().contains("onboard first"),
            "raw cause text must not be copied into model-visible output"
        );
    }

    #[test]
    fn parse_profile_set_input_trims_handle_and_optional_bio() {
        let parsed = parse_profile_set_input(&json!({
            "display_handle": "  pilot_zaki  ",
            "bio": "  Repair loop enjoyer  ",
            "confirmed": true
        }))
        .unwrap();
        assert_eq!(parsed.display_handle, "pilot_zaki");
        assert_eq!(parsed.bio.as_deref(), Some("Repair loop enjoyer"));
        assert!(parsed.confirmed);
    }

    #[test]
    fn parse_profile_set_input_defaults_confirmed_to_false() {
        let parsed = parse_profile_set_input(&json!({"display_handle": "pilot_zaki"})).unwrap();
        assert!(!parsed.confirmed);
    }

    #[tokio::test]
    async fn dispatch_profile_set_without_confirmed_returns_consent_required_no_write() {
        // confirmed=false short-circuits before the enrollment check and any
        // network call — the public-attribution opt-in is a hard input gate,
        // mirroring dispatch_onboard.
        let request = test_request(json!({
            "display_handle": "pilot_zaki",
            "bio": "Trace Commons pilot contributor",
        }));
        let result = dispatch_profile_set(&request).await.unwrap();
        assert_eq!(result["updated"], json!(false));
        assert_eq!(result["consent_required"], json!(true));
        assert!(
            result["message"]
                .as_str()
                .is_some_and(|m| m.contains("confirmed=true"))
        );
    }

    #[test]
    fn parse_profile_set_input_rejects_missing_handle() {
        let error = parse_profile_set_input(&json!({"bio": "hello"})).unwrap_err();
        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::InputEncode));
    }

    #[test]
    fn profile_set_success_value_keeps_scope_boundary_visible() {
        let input = ProfileSetToolInput {
            display_handle: "pilot_zaki".to_string(),
            bio: Some("Trace Commons pilot contributor".to_string()),
            confirmed: true,
        };
        let v = profile_set_success_value(&input);
        assert_eq!(v["updated"], json!(true));
        assert_eq!(v["display_handle"], json!("pilot_zaki"));
        assert_eq!(
            v["bio"],
            json!(Some("Trace Commons pilot contributor".to_string()))
        );
        // No fixed profile URL is surfaced — the profile lives on the user's
        // enrolled (possibly self-hosted) issuer, so naming a fixed origin
        // would misdirect a self-hosted user.
        assert!(
            v.get("profile_url").is_none(),
            "no fixed profile URL may be surfaced on the success result"
        );
        assert!(
            !serde_json::to_string(&v)
                .unwrap()
                .contains("tracecommons.ai"),
            "the hardcoded tracecommons.ai origin must not appear in output"
        );
        assert!(
            v["message"]
                .as_str()
                .is_some_and(|message| message.contains("leaderboard"))
        );
    }

    #[test]
    fn profile_set_error_maps_validation_without_raw_error() {
        let v = profile_set_error_value(&CommunityProfileError::InvalidProfile(
            "community profile handle must be at least 3 characters".to_string(),
        ));
        assert_eq!(v["updated"], json!(false));
        assert_eq!(v["error_code"], json!("InvalidProfile"));
        let serialized = serde_json::to_string(&v).unwrap();
        assert!(
            !serialized.contains("at least 3"),
            "raw validation text should not be copied into model-visible output"
        );
    }

    #[tokio::test]
    async fn dispatch_profile_token_without_confirmed_returns_consent_required_no_mint() {
        // No confirmed=true: the hard consent gate must short-circuit before
        // any token is minted or persisted (a bearer credential), even though
        // the runtime Ask gate could be auto-approved in local-yolo.
        let request = test_request(json!({}));
        let result = dispatch_profile_token(&request).await.unwrap();
        assert_eq!(result["minted"], json!(false));
        assert_eq!(result["consent_required"], json!(true));
        assert!(
            result.get("token_delivery").is_none(),
            "no token may be minted before consent"
        );
    }

    #[tokio::test]
    async fn dispatch_profile_token_without_enrollment_returns_onboard_guidance() {
        let request = test_request(json!({ "confirmed": true }));
        let result = dispatch_profile_token(&request).await.unwrap();
        assert_eq!(result["minted"], json!(false));
        assert_eq!(result["error_code"], json!("NotEnrolled"));
        let message = result["message"].as_str().unwrap();
        assert!(
            message.contains("Onboard with the operator invite link first"),
            "agent-visible guidance should direct the user to onboard first"
        );
    }

    #[tokio::test]
    async fn dispatch_profile_set_without_enrollment_returns_onboard_guidance() {
        let request = test_request(json!({
            "display_handle": "pilot_zaki",
            "bio": "Trace Commons pilot contributor",
            "confirmed": true
        }));
        let result = dispatch_profile_set(&request).await.unwrap();
        assert_eq!(result["updated"], json!(false));
        assert_eq!(result["error_code"], json!("NotEnrolled"));
        let message = result["message"].as_str().unwrap();
        assert!(
            message.contains("Onboard with the operator invite link first"),
            "agent-visible guidance should direct the user to onboard first"
        );
    }
}
