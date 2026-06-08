//! First-party Trace Commons capabilities: onboard and status.
//!
//! `trace_commons.onboard` drives the operator-invite enrollment flow.
//! `trace_commons.status` is a read-only policy inspector.
//!
//! Both are model-visible; the agent-facing guidance lives in the
//! prompt_doc_ref files (prompts/builtin/trace-commons-{onboard,status}.md).

use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode, ResourceEstimate, ResourceProfile};
use ironclaw_reborn_traces::contribution::{
    StandingTraceContributionPolicy, TraceUploadAuthMode, read_trace_policy_for_scope,
};
use ironclaw_reborn_traces::onboarding::{
    OnboardConsents, OnboardError, OnboardOutcome, protocol::OnboardErrorCode,
};
use serde_json::{Value, json};

use crate::FirstPartyCapabilityError;
use crate::FirstPartyCapabilityRequest;

use super::{
    FIRST_PARTY_DEFAULT_OUTPUT_BYTES, first_party_capability_manifest, input_error,
    resource_profile,
};

pub const TRACE_COMMONS_ONBOARD_CAPABILITY_ID: &str = "builtin.trace_commons.onboard";
pub const TRACE_COMMONS_STATUS_CAPABILITY_ID: &str = "builtin.trace_commons.status";

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

// ── Input parsing ─────────────────────────────────────────────────────────────

struct OnboardToolInput {
    invite_url: String,
    consents: OnboardConsents,
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

    let scope = request.scope.user_id.as_str().to_string();
    match ironclaw_reborn_traces::onboarding::onboard(&scope, &input.invite_url, input.consents)
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
    the score threshold. Opt out anytime with 'ironclaw traces opt-out'."
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_filesystem::LocalFilesystem;
    use ironclaw_host_api::{CapabilityId, ResourceEstimate, ResourceScope};
    use ironclaw_reborn_traces::contribution::StandingTraceContributionPolicy;
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
                process: Arc::new(NoopProcessPort),
                secret_store: None,
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
}
