use super::*;

#[test]
fn capability_failure_display_summary_renders_invalid_input_issues() {
    let detail = CapabilityFailureDetail::InvalidInput {
        issues: vec![
            CapabilityInputIssue {
                path: "schedule.kind".to_string(),
                code: DispatchInputIssueCode::MissingRequired,
                expected: Some("cron or once".to_string()),
                received: Some("super-secret-raw-value".to_string()),
                schema_path: None,
            },
            CapabilityInputIssue {
                path: "schedule.timezone".to_string(),
                code: DispatchInputIssueCode::InvalidValue,
                expected: None,
                received: None,
                schema_path: None,
            },
        ],
    };
    let summary = failure_display_summary("tool input failed validation", &detail)
        .expect("invalid input renders a summary");
    assert!(summary.starts_with("Invalid input:"));
    assert!(summary.contains("schedule.kind — missing required field (expected cron or once)"));
    assert!(summary.contains("schedule.timezone — invalid value"));
    // `received` echoes raw tool input and must never reach a display surface.
    assert!(!summary.contains("super-secret-raw-value"));
}

#[test]
fn capability_failure_display_summary_uses_safe_summary_without_issues() {
    // The `json` builtin reports invalid_input with a descriptive message
    // but no structured issues; that message must reach the preview.
    assert_eq!(
        failure_display_summary(
            "invalid JSON: expected value at line 1 column 1",
            &CapabilityFailureDetail::Diagnostic {
                text: ModelDiagnostic::unavailable().into_inner(),
            },
        )
        .as_deref(),
        Some("invalid JSON: expected value at line 1 column 1")
    );
}

#[test]
fn capability_failure_display_summary_skips_unsafe_input_issue_fields() {
    let detail = CapabilityFailureDetail::InvalidInput {
        issues: vec![CapabilityInputIssue {
            path: "payload</script>".to_string(),
            code: DispatchInputIssueCode::InvalidValue,
            expected: Some("safe".to_string()),
            received: None,
            schema_path: None,
        }],
    };

    assert_eq!(
        failure_display_summary("input schema validation failed", &detail).as_deref(),
        Some("input schema validation failed")
    );
}

#[test]
fn capability_failure_display_summary_skips_sensitive_input_issue_fields() {
    let detail = CapabilityFailureDetail::InvalidInput {
        issues: vec![CapabilityInputIssue {
            path: "secret_api_key".to_string(),
            code: DispatchInputIssueCode::TypeMismatch,
            expected: Some("password string".to_string()),
            received: None,
            schema_path: None,
        }],
    };

    assert_eq!(
        failure_display_summary("input schema validation failed", &detail).as_deref(),
        Some("input schema validation failed")
    );
}

#[test]
fn capability_input_issue_display_text_rejects_sensitive_marker_variants() {
    for value in [
        "x-api-key",
        "accessToken",
        "auth_token",
        "toolInput",
        "secret_api_key",
    ] {
        assert_eq!(capability_input_issue_display_text(value), None, "{value}");
    }
}

#[test]
fn capability_failure_display_summary_is_none_for_generic_placeholder() {
    assert!(
        failure_display_summary(
            "capability invocation failed",
            &CapabilityFailureDetail::Diagnostic {
                text: ModelDiagnostic::unavailable().into_inner(),
            },
        )
        .is_none()
    );
}
