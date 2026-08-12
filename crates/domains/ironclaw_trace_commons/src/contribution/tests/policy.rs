//! The standing contribution policy: its serde contract (including
//! back-compat with policy JSON written before a field existed) and the
//! auth-mode round trip.

use crate::contribution::*;

#[test]
fn standing_policy_serde_back_compat_when_invite_code_missing() {
    // Existing policy files written before the invite_code field landed
    // must continue to parse unchanged.
    let legacy_json = r#"{
        "enabled": true,
        "ingestion_endpoint": "https://example/v1/traces",
        "bearer_token_env": "IRONCLAW_TRACE_SUBMIT_TOKEN",
        "upload_token_issuer_url": "https://issuer.example/v1/trace-upload-claim",
        "upload_token_issuer_allowed_hosts": ["issuer.example"],
        "upload_token_audience": "trace-commons",
        "upload_token_tenant_id": "tenant-a",
        "upload_token_workload_token_env": "IRONCLAW_TRACE_WORKLOAD_TOKEN",
        "upload_token_issuer_timeout_ms": 7000,
        "include_message_text": false,
        "include_tool_payloads": false,
        "auto_submit_failed_traces": true,
        "auto_submit_high_value_traces": true,
        "selected_tools": [],
        "require_manual_approval_when_pii_detected": true,
        "min_submission_score": 0.35,
        "credit_notice_interval_hours": 168,
        "default_scope": "debugging_evaluation"
    }"#;
    let policy: StandingTraceContributionPolicy =
        serde_json::from_str(legacy_json).expect("legacy policy parses");
    assert!(policy.upload_token_invite_code.is_none());
    assert!(policy.enabled);
}
#[test]
fn standing_policy_serde_round_trips_invite_code_when_set() {
    let policy =
        StandingTraceContributionPolicy::default().set_upload_token_invite_code("INV-PILOT-001");
    let serialized = serde_json::to_string(&policy).expect("serializes");
    assert!(
        serialized.contains("\"upload_token_invite_code\":\"INV-PILOT-001\""),
        "serialized policy carries invite code: {serialized}"
    );
    let round: StandingTraceContributionPolicy =
        serde_json::from_str(&serialized).expect("round trips");
    assert_eq!(
        round.upload_token_invite_code.as_deref(),
        Some("INV-PILOT-001")
    );
}
#[test]
fn standing_policy_serde_omits_invite_code_when_none() {
    // skip_serializing_if keeps existing-shape policies byte-identical
    // for deployments that never configured an invite code.
    let policy = StandingTraceContributionPolicy::default();
    let serialized = serde_json::to_string(&policy).expect("serializes");
    assert!(
        !serialized.contains("upload_token_invite_code"),
        "default policy must not emit upload_token_invite_code: {serialized}"
    );
}
#[test]
fn legacy_policy_json_defaults_to_workload_token_env_auth() {
    // Take the default policy's JSON and strip the two NEW fields to simulate
    // a pre-upgrade policy file on disk.
    let mut legacy = serde_json::to_value(StandingTraceContributionPolicy::default()).unwrap();
    let obj = legacy.as_object_mut().unwrap();
    obj.remove("auth_mode");
    obj.remove("device_key_id");
    let policy: StandingTraceContributionPolicy = serde_json::from_value(legacy).unwrap();
    assert_eq!(policy.auth_mode, TraceUploadAuthMode::WorkloadTokenEnv);
    assert!(policy.device_key_id.is_none());
}
#[test]
fn device_key_policy_round_trips() {
    let policy = StandingTraceContributionPolicy::default()
        .set_auth_mode(TraceUploadAuthMode::DeviceKey)
        .set_device_key_id("sha256:abc".to_string());
    let json = serde_json::to_value(&policy).unwrap();
    assert_eq!(json["auth_mode"], "device_key");
    let back: StandingTraceContributionPolicy = serde_json::from_value(json).unwrap();
    assert_eq!(back.auth_mode, TraceUploadAuthMode::DeviceKey);
    assert_eq!(back.device_key_id.as_deref(), Some("sha256:abc"));
}
