use ironclaw_host_runtime::{ExposureBoundary, NoExposureGuard, NoExposureViolation};
use serde_json::json;

#[test]
fn host_no_exposure_guard_blocks_default_secret_patterns() {
    let guard = NoExposureGuard::new();

    let error = guard
        .check_text(
            ExposureBoundary::ModelVisibleToolOutput,
            "tool returned sk-proj-test1234567890abcdefghij",
        )
        .expect_err("blocked default detector match should fail");

    assert_eq!(error.code(), NoExposureViolation::CODE);
    assert_eq!(error.boundary(), ExposureBoundary::ModelVisibleToolOutput);
    assert!(
        !error
            .to_string()
            .contains("sk-proj-test1234567890abcdefghij")
    );
}

#[test]
fn host_no_exposure_guard_blocks_private_keys_with_stable_error() {
    let guard = NoExposureGuard::new();
    let private_key = "-----BEGIN PRIVATE KEY-----\nabc123\n-----END PRIVATE KEY-----";

    let error = guard
        .check_text(ExposureBoundary::PublicApi, private_key)
        .expect_err("private keys should block");

    assert_eq!(error.code(), NoExposureViolation::CODE);
    assert_eq!(error.boundary(), ExposureBoundary::PublicApi);
    assert!(!error.to_string().contains("PRIVATE KEY"));
    assert!(!error.to_string().contains("abc123"));
}

#[test]
fn host_no_exposure_guard_checks_json_recursively() {
    let guard = NoExposureGuard::new();
    let value = json!({
        "message": "token sk-proj-test1234567890abcdefghij",
        "nested": ["ok"]
    });

    let error = guard
        .check_json(ExposureBoundary::SseEvent, value)
        .expect_err("blocked json secret should fail");

    assert_eq!(error.code(), NoExposureViolation::CODE);
    assert_eq!(error.boundary(), ExposureBoundary::SseEvent);
    assert!(
        !error
            .to_string()
            .contains("sk-proj-test1234567890abcdefghij")
    );
}

#[test]
fn host_no_exposure_guard_blocks_redactable_http_request_matches() {
    let guard = NoExposureGuard::new();

    let error = guard
        .check_http_request(
            ExposureBoundary::PublicApi,
            "https://api.example.test/run",
            &[],
            Some(b"{\"authorization\":\"Bearer abcdefghij0123456789\"}"),
        )
        .expect_err("redactable bearer token should not leave over HTTP egress");

    assert_eq!(error.code(), NoExposureViolation::CODE);
    assert_eq!(error.boundary(), ExposureBoundary::PublicApi);
    assert!(!error.to_string().contains("Bearer abcdefghij0123456789"));
}

#[test]
fn host_no_exposure_guard_scans_http_header_names() {
    let guard = NoExposureGuard::new();

    let error = guard
        .check_http_request(
            ExposureBoundary::PublicApi,
            "https://api.example.test/run",
            &[(
                "sk-proj-test1234567890abcdefghij".to_string(),
                "value".to_string(),
            )],
            None,
        )
        .expect_err("credential-shaped header names should fail closed");

    assert_eq!(error.code(), NoExposureViolation::CODE);
    assert_eq!(error.boundary(), ExposureBoundary::PublicApi);
    assert!(
        !error
            .to_string()
            .contains("sk-proj-test1234567890abcdefghij")
    );
}
