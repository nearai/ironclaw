//! First-party Trace Commons capabilities: onboard, status, credits, profile token, and profile set.
//!
//! `trace_commons.onboard` drives the operator-invite enrollment flow.
//! `trace_commons.status` is a read-only policy inspector.
//! `trace_commons.credits` is a read-only credit balance reporter.
//! `trace_commons.profile_token` mints a short-lived public-attribution token.
//! `trace_commons.profile_set` updates the public community profile directly.
//!
//! All five are model-visible.

use std::{panic::AssertUnwindSafe, sync::Arc};

use async_trait::async_trait;
use futures_util::FutureExt as _;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    CapabilityId, EffectKind, NetworkMethod, NetworkPolicy, PermissionMode, ResourceEstimate,
    ResourceProfile, ResourceScope, RuntimeDispatchErrorKind, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeKind,
};
use ironclaw_reborn_traces::contribution::{
    ProfileAttributionToken, StandingTraceContributionPolicy, TraceCreditReport,
    TraceUploadAuthMode, mint_profile_attribution_token_for_scope, read_trace_policy_for_scope,
    set_community_profile_for_scope,
};
use ironclaw_reborn_traces::onboarding::{
    OnboardConsents, OnboardError, OnboardHttpResponse, OnboardOutcome, OnboardingHttpSink,
    protocol::OnboardErrorCode,
};
use serde_json::{Value, json};

use crate::FirstPartyCapabilityError;
use crate::FirstPartyCapabilityRequest;

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

// ── Manifest helpers ─────────────────────────────────────────────────────────

pub(super) fn onboard_manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
        "Enroll this IronClaw in Trace Commons using an operator-issued invite link. \
         ONLY call after the user has explicitly (1) confirmed they want to contribute \
         redacted traces and (2) chosen whether to include redacted message text and \
         redacted tool payloads. Pass confirmed=true only when both consents were given \
         in this conversation.",
        vec![EffectKind::Network, EffectKind::ExternalWrite],
        PermissionMode::Ask,
        Some(ResourceProfile {
            default_estimate: ResourceEstimate {
                wall_clock_ms: Some(15_000),
                // The surface contract requires every visible capability to
                // advertise an output_bytes estimate.
                output_bytes: Some(FIRST_PARTY_DEFAULT_OUTPUT_BYTES),
                ..ResourceEstimate::default()
            },
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
         submit traces.",
        vec![
            EffectKind::ReadFilesystem,
            EffectKind::Network,
            EffectKind::ExternalWrite,
        ],
        PermissionMode::Ask,
        Some(ResourceProfile {
            default_estimate: ResourceEstimate {
                wall_clock_ms: Some(10_000),
                output_bytes: Some(FIRST_PARTY_DEFAULT_OUTPUT_BYTES),
                ..ResourceEstimate::default()
            },
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
        PermissionMode::Allow,
        Some(ResourceProfile {
            default_estimate: ResourceEstimate {
                wall_clock_ms: Some(15_000),
                output_bytes: Some(FIRST_PARTY_DEFAULT_OUTPUT_BYTES),
                ..ResourceEstimate::default()
            },
            hard_ceiling: None,
        }),
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
    let bio = input
        .get("bio")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|bio| !bio.is_empty())
        .map(str::to_string);
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

    let scope = request.scope.user_id.as_str().to_string();
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
        OnboardError::DeviceKey(_) | OnboardError::Persist { .. } => (
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
    let scope = request.scope.user_id.as_str().to_string();
    // A missing or unreadable policy is a normal "not enrolled" state — map
    // read errors to a soft fallback value, not a FirstPartyCapabilityError.
    let policy = match read_trace_policy_for_scope(Some(scope.as_str())) {
        Ok(p) => p,
        Err(_) => {
            return Ok(json!({
                "enrolled": false,
                "error": "could not read policy"
            }));
        }
    };
    Ok(format_status(&policy))
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
    let scope = request.scope.user_id.as_str().to_string();
    // A missing or unreadable submissions file is a normal "nothing submitted yet"
    // state — map read errors to a soft fallback value, not a FirstPartyCapabilityError.
    match ironclaw_reborn_traces::contribution::read_local_trace_records_for_scope(Some(
        scope.as_str(),
    )) {
        Ok(records) => Ok(format_credits(
            &ironclaw_reborn_traces::contribution::trace_credit_report(&records),
        )),
        Err(_) => Ok(json!({
            "enrolled_or_active": false,
            "message": "No local Trace Commons submission records found for this user."
        })),
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
    let scope = request.scope.user_id.as_str().to_string();
    match mint_profile_attribution_token_for_scope(Some(scope.as_str())).await {
        Ok(token) => Ok(format_profile_token(&token)),
        Err(error) => Ok(profile_token_error_value(error.to_string())),
    }
}

fn format_profile_token(token: &ProfileAttributionToken) -> Value {
    json!({
        "minted": true,
        "token_type": "Bearer",
        "access_token": token.access_token.as_str(),
        "expires_at": token.expires_at.as_ref().map(|dt| dt.to_rfc3339()),
        "expires_in": token.expires_in,
        "consent_scope": "public_attribution",
        "allowed_uses": [],
        "profile_url": "https://tracecommons.ai/profile",
        "message": "Prefer asking the agent to set your public profile directly with a pseudonymous handle. \
    For browser/manual setup only, paste access_token exactly as shown into https://tracecommons.ai/profile \
    without adding a Bearer prefix. This token is short-lived and only authorizes public profile management."
    })
}

fn profile_token_error_value(error: String) -> Value {
    let (error_code, message) = if error.contains("not enrolled in Trace Commons")
        || error.contains("could not read policy")
    {
        (
            "NotEnrolled",
            "Trace Commons enrollment was not found for this user. Onboard with the operator invite link first.",
        )
    } else if error.contains("issuer URL is not configured") {
        (
            "IssuerNotConfigured",
            "Trace Commons enrollment is missing the upload-claim issuer URL. Re-run onboarding with a fresh invite.",
        )
    } else if error.contains("device key") {
        (
            "DeviceKeyUnavailable",
            "Trace Commons device-key state is incomplete. Re-run onboarding with a fresh invite.",
        )
    } else if error.contains("refused") {
        (
            "IssuerRefused",
            "The Trace Commons issuer refused to mint a profile token. Ask the operator to check invite/device-key status.",
        )
    } else {
        (
            "ProfileTokenMintFailed",
            "Could not mint a Trace Commons profile token. Check enrollment status and retry.",
        )
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
    // separate public-attribution opt-in, and this capability is exempt from
    // approval gates in local-dev policy — so the hard invariant lives here,
    // mirroring dispatch_onboard. Never reach the network without explicit
    // per-conversation confirmation.
    if !input.confirmed {
        return Ok(json!({
            "updated": false,
            "consent_required": true,
            "message": "Publishing or updating the public Trace Commons community profile is a \
        separate public-attribution opt-in. Confirm the display handle (and optional bio) with \
        the user first, then call again with confirmed=true."
        }));
    }

    let scope = request.scope.user_id.as_str().to_string();
    match read_trace_policy_for_scope(Some(scope.as_str())) {
        Ok(policy) if policy.enabled => {}
        Ok(_) => {
            return Ok(profile_set_error_value(
                "not enrolled in Trace Commons".to_string(),
            ));
        }
        Err(error) => return Ok(profile_set_error_value(error.to_string())),
    }
    match set_community_profile_for_scope(
        Some(scope.as_str()),
        &input.display_handle,
        input.bio.as_deref(),
    )
    .await
    {
        Ok(()) => Ok(profile_set_success_value(&input)),
        Err(error) => Ok(profile_set_error_value(error.to_string())),
    }
}

fn profile_set_success_value(input: &ProfileSetToolInput) -> Value {
    json!({
        "updated": true,
        "display_handle": input.display_handle.as_str(),
        "bio": input.bio.as_deref(),
        "profile_url": "https://tracecommons.ai/profile",
        "message": "Your public Trace Commons profile handle is set. It may appear on the leaderboard after the next snapshot.",
    })
}

fn profile_set_error_value(error: String) -> Value {
    let (error_code, message) = if error.contains("not enrolled in Trace Commons")
        || error.contains("could not read policy")
    {
        (
            "NotEnrolled",
            "Trace Commons enrollment was not found for this user. Onboard with the operator invite link first.",
        )
    } else if error.contains("community profile handle") || error.contains("community profile bio")
    {
        (
            "InvalidProfile",
            "Choose a pseudonymous handle 3-32 characters long using ASCII letters, digits, '-' or '_'. Bio must be at most 280 bytes.",
        )
    } else if error.contains("issuer URL is not configured") {
        (
            "IssuerNotConfigured",
            "Trace Commons enrollment is missing the upload-claim issuer URL. Re-run onboarding with a fresh invite.",
        )
    } else if error.contains("device key") {
        (
            "DeviceKeyUnavailable",
            "Trace Commons device-key state is incomplete. Re-run onboarding with a fresh invite.",
        )
    } else if error.contains("refused") || error.contains("rejected") {
        (
            "IssuerRefused",
            "Trace Commons refused the public profile update. Ask the operator to check invite/device-key status.",
        )
    } else {
        (
            "ProfileSetFailed",
            "Could not update the Trace Commons public profile. Check enrollment status and retry.",
        )
    };
    json!({
        "updated": false,
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
    use ironclaw_filesystem::LocalFilesystem;
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
            capability_id: CapabilityId::new(TRACE_COMMONS_ONBOARD_CAPABILITY_ID).unwrap(),
            scope: ResourceScope::system(),
            estimate: ResourceEstimate::default(),
            mounts: None,
            services: InvocationServices {
                filesystem: Arc::new(LocalFilesystem::new()),
                runtime_http_egress: None,
                tool_call_http_egress: None,
                process: Arc::new(NoopProcessPort),
                secret_store: None,
                audit_sink: None,
                unsafe_raw_diagnostics_allowed: false,
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
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            auth_mode: TraceUploadAuthMode::DeviceKey,
            upload_token_tenant_id: Some("tenant-z".to_string()),
            include_message_text: true,
            include_tool_payloads: false,
            ingestion_endpoint: Some("https://ingest.example.com".to_string()),
            ..StandingTraceContributionPolicy::default()
        };
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
    fn format_profile_token_reports_raw_token_and_scope_boundary() {
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
        assert_eq!(v["access_token"], json!("eyJ.profile.token"));
        assert_eq!(v["consent_scope"], json!("public_attribution"));
        assert_eq!(v["allowed_uses"], json!([]));
        assert_eq!(v["profile_url"], json!("https://tracecommons.ai/profile"));
        let message = v["message"].as_str().unwrap();
        assert!(
            message.contains("agent to set your public profile directly"),
            "message must prefer direct agent profile setup"
        );
        assert!(
            message.contains("without adding a Bearer prefix"),
            "message must still preserve the browser/manual token fallback"
        );
    }

    #[test]
    fn profile_token_error_maps_missing_enrollment_without_raw_error() {
        let v =
            profile_token_error_value("not enrolled in Trace Commons - onboard first".to_string());
        assert_eq!(v["minted"], json!(false));
        assert_eq!(v["error_code"], json!("NotEnrolled"));
        let serialized = serde_json::to_string(&v).unwrap();
        assert!(
            !serialized.contains("onboard first"),
            "raw anyhow text should not be copied into model-visible output"
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
        assert_eq!(v["profile_url"], json!("https://tracecommons.ai/profile"));
        assert!(
            v["message"]
                .as_str()
                .is_some_and(|message| message.contains("leaderboard"))
        );
    }

    #[test]
    fn profile_set_error_maps_validation_without_raw_error() {
        let v = profile_set_error_value(
            "community profile handle must be at least 3 characters".to_string(),
        );
        assert_eq!(v["updated"], json!(false));
        assert_eq!(v["error_code"], json!("InvalidProfile"));
        let serialized = serde_json::to_string(&v).unwrap();
        assert!(
            !serialized.contains("at least 3"),
            "raw validation text should not be copied into model-visible output"
        );
    }

    #[tokio::test]
    async fn dispatch_profile_token_without_enrollment_returns_onboard_guidance() {
        let request = test_request(json!({}));
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
