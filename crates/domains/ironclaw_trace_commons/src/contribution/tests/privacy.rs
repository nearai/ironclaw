//! Policy preflight, deterministic redaction, the privacy-filter adapter and its canary, and per-tool payload redaction.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ironclaw_llm::recording::{TraceFile, TraceResponse};

use crate::contribution::*;
use ironclaw_llm::recording::TraceStep;

use super::support::*;

#[test]
fn trace_policy_preflight_gates_queue_and_submit_intents() {
    let disabled = StandingTraceContributionPolicy::default();
    assert_eq!(
        preflight_trace_contribution_policy(&disabled, TraceContributionAcceptance::PreviewOnly),
        Ok(())
    );
    assert_eq!(
        preflight_trace_contribution_policy(
            &disabled,
            TraceContributionAcceptance::QueueFromPreview
        ),
        Err(TraceContributionPolicyRejection::OptInDisabled)
    );
    assert_eq!(
        preflight_trace_contribution_policy(&disabled, TraceContributionAcceptance::ManualSubmit),
        Err(TraceContributionPolicyRejection::OptInDisabled)
    );
    assert_eq!(
        preflight_trace_contribution_policy(
            &disabled,
            TraceContributionAcceptance::AutonomousSubmit
        ),
        Err(TraceContributionPolicyRejection::OptInDisabled)
    );

    let mut missing_endpoint = StandingTraceContributionPolicy::default().set_enabled(true);
    assert_eq!(
        preflight_trace_contribution_policy(
            &missing_endpoint,
            TraceContributionAcceptance::ManualSubmit
        ),
        Err(TraceContributionPolicyRejection::EndpointMissing)
    );

    missing_endpoint.ingestion_endpoint = Some("https://trace.example/v1/traces".to_string());
    assert_eq!(
        preflight_trace_contribution_policy(
            &missing_endpoint,
            TraceContributionAcceptance::ManualSubmit
        ),
        Ok(())
    );
}
/// Lane-2 safety pin (extension-runtime DEL-8). The trace redaction
/// classifier keys the payload-redaction profile (and the external-write
/// side-effect level) off tool-name keywords; the vendor keywords are a
/// genuine safety DENYLIST, not extension routing. It is deliberately a
/// SUPERSET of the bundled package inventory — it must also cover
/// non-package messaging/issue-tracker tools such as signal, discord, and
/// gitlab — so it cannot be sourced from the inventory without weakening
/// redaction. This locks the mapping so a future "de-hardcode the vendor
/// names" cleanup cannot silently drop a keyword and stop redacting a
/// tool's sensitive payload. The `contribution.rs` PATH_TERM_COLLISIONS
/// carve-out in
/// `crates/ironclaw_architecture_tests/tests/reborn_extension_specificity.rs`
/// documents why the names stay here.
#[test]
fn tool_payload_redaction_profile_is_a_safety_denylist_not_inventory_routing() {
    // Package-vendor keywords select the profile whose rules redact that
    // payload shape.
    assert!(matches!(
        tool_payload_profile("slack.send_message"),
        Some(ToolPayloadProfile::Messaging)
    ));
    assert!(matches!(
        tool_payload_profile("gmail.send_email"),
        Some(ToolPayloadProfile::Email)
    ));
    assert!(matches!(
        tool_payload_profile("github.create_issue"),
        Some(ToolPayloadProfile::IssueTracker)
    ));
    // Non-inventory keywords must ALSO classify — dropping them (as
    // sourcing the set from the package inventory would) silently stops
    // redacting those tools' payloads.
    assert!(matches!(
        tool_payload_profile("signal.send"),
        Some(ToolPayloadProfile::Messaging)
    ));
    assert!(matches!(
        tool_payload_profile("discord.post_message"),
        Some(ToolPayloadProfile::Messaging)
    ));
    assert!(matches!(
        tool_payload_profile("gitlab.open_merge_request"),
        Some(ToolPayloadProfile::IssueTracker)
    ));

    // A `slack`-named send is an external write (the safety signal),
    // distinct from a local write.
    assert!(matches!(
        classify_tool_side_effect("slack.send_message"),
        SideEffectLevel::ExternalWrite
    ));
    assert!(matches!(
        classify_tool_side_effect("file.write"),
        SideEffectLevel::LocalWrite
    ));

    // Drive the production caller: a messaging tool's content field is
    // redacted; a tool the classifier does not recognize passes through
    // untouched (the control proving the keyword gates the redaction).
    let payload = serde_json::json!({ "message": "meet me at 5", "channel": "C42" });
    let mut report = RedactionReport::default();
    let redacted = redact_tool_specific_payload(Some("slack.send_message"), &payload, &mut report);
    assert_ne!(
        redacted.get("message"),
        payload.get("message"),
        "a messaging tool's message content must be redacted"
    );

    let mut report = RedactionReport::default();
    let untouched = redact_tool_specific_payload(Some("weather.forecast"), &payload, &mut report);
    assert_eq!(
        untouched, payload,
        "a tool with no payload profile must pass through unredacted"
    );
}
#[tokio::test]
async fn metadata_only_recorded_trace_omits_message_text_and_tool_arguments() {
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let envelope = DeterministicTraceRedactor::with_known_path_prefixes([PathBuf::from(
        "/Users/alice/project",
    )])
    .redact_trace(raw)
    .await
    .expect("redaction should succeed");

    let json = serde_json::to_string(&envelope).expect("envelope serializes");
    assert!(!json.contains("alice@example.com"));
    assert!(!json.contains("abcdefghijklmnopqrstuvwxyz"));
    assert!(!json.contains("/Users/alice/project"));
    assert!(json.contains("\"tool_name\":\"http\""));
    assert!(!envelope.consent.message_text_included);
    assert!(!envelope.consent.tool_payloads_included);
    assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::Low);
}
#[tokio::test]
async fn text_and_payload_preview_redacts_paths_and_sensitive_fields() {
    let options = RecordedTraceContributionOptions::default()
        .set_include_message_text(true)
        .set_include_tool_payloads(true);
    let raw = RawTraceContribution::from_recorded_trace(&sample_trace(), options);
    let envelope = DeterministicTraceRedactor::with_known_path_prefixes([PathBuf::from(
        "/Users/alice/project",
    )])
    .redact_trace(raw)
    .await
    .expect("redaction should succeed");

    let json = serde_json::to_string(&envelope).expect("envelope serializes");
    assert!(json.contains("<PRIVATE_LOCAL_PATH_"));
    assert!(json.contains("<PRIVATE_EMAIL_"));
    assert!(json.contains("[REDACTED]"));
    assert!(!json.contains("/Users/alice/project"));
    assert!(!json.contains("alice@example.com"));
    assert!(!json.contains("abcdefghijklmnopqrstuvwxyz"));
    assert_eq!(
        envelope.privacy.redaction_counts.get("local_path"),
        Some(&2)
    );
    assert_eq!(
        envelope.privacy.redaction_counts.get("sensitive_field"),
        Some(&1)
    );
    assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::Medium);
}
#[test]
fn deterministic_text_redactor_redacts_generic_local_paths() {
    let redactor = DeterministicTraceRedactor::new(Vec::new());
    let (redacted, report) =
        redactor.redact_text("read /tmp/ironclaw/private/token.txt before upload");

    assert_eq!(redacted, "read <PRIVATE_LOCAL_PATH_1> before upload");
    assert_eq!(report.counts.get("local_path"), Some(&1));
}
#[test]
fn stable_placeholders_preserve_entity_distinctions() {
    let redactor = DeterministicTraceRedactor::new(Vec::new());
    let (redacted, report) = redactor.redact_text(
        "Email alice@example.com, copy bob@example.com, then follow up with alice@example.com.",
    );

    assert!(redacted.contains("<PRIVATE_EMAIL_1>"));
    assert!(redacted.contains("<PRIVATE_EMAIL_2>"));
    assert_eq!(redacted.matches("<PRIVATE_EMAIL_1>").count(), 2);
    assert_eq!(redacted.matches("<PRIVATE_EMAIL_2>").count(), 1);
    assert!(!redacted.contains("alice@example.com"));
    assert!(!redacted.contains("bob@example.com"));
    assert_eq!(report.counts.get("private_email"), Some(&3));
    assert!(
        report
            .pii_labels_present
            .contains(&"private_email".to_string())
    );
}
#[test]
fn privacy_filter_summary_shape_cannot_serialize_original_span_text() {
    let summary = SafePrivacyFilterSummary {
        schema_version: 1,
        output_mode: "redacted_text_only".to_string(),
        span_count: 2,
        by_label: BTreeMap::from([("private_email".to_string(), 2)]),
        decoded_mismatch: false,
    };

    let json = serde_json::to_string(&summary).expect("summary serializes");
    assert!(json.contains("private_email"));
    assert!(!json.contains("alice@example.com"));
    assert!(!json.contains("detected_spans"));
    assert!(!json.contains("\"text\""));
}
#[test]
fn privacy_filter_output_adapter_strips_raw_span_text() {
    let output = serde_json::json!({
        "schema_version": 1,
        "text": "Email alice@example.com with secret sk-test",
        "redacted_text": "Email <PRIVATE_EMAIL> with <SECRET>",
        "detected_spans": [
            {"label": "private_email", "start": 6, "end": 23, "text": "alice@example.com"},
            {"label": "secret", "start": 36, "end": 43, "text": "sk-test"}
        ]
    });

    let safe = safe_privacy_filter_redaction_from_output(&output).expect("privacy output parses");
    let json = serde_json::to_string(&safe).expect("safe output serializes");

    assert_eq!(safe.redacted_text, "Email <PRIVATE_EMAIL> with <SECRET>");
    assert_eq!(safe.summary.span_count, 2);
    assert_eq!(safe.summary.by_label.get("private_email"), Some(&1));
    assert!(safe.report.blocked_secret_detected);
    assert!(!json.contains("alice@example.com"));
    assert!(!json.contains("sk-test"));
    assert!(!json.contains("detected_spans"));
}
#[test]
fn privacy_filter_output_adapter_maps_unsafe_labels_without_leaking_them() {
    let output = serde_json::json!({
        "schema_version": 1,
        "redacted_text": "Email <PRIVATE_EMAIL> with <SECRET>",
        "detected_spans": [
            {"label": "alice@example.com", "text": "alice@example.com"},
            {"type": "/Users/alice/.ssh/id_rsa", "text": "/Users/alice/.ssh/id_rsa"},
            {"entity_type": "sk-test-raw-token", "text": "sk-test-raw-token"}
        ]
    });

    let safe = safe_privacy_filter_redaction_from_output(&output).expect("privacy output parses");
    let json = serde_json::to_string(&safe).expect("safe output serializes");

    assert_eq!(safe.summary.by_label.get("unknown"), Some(&3));
    assert_eq!(safe.report.counts.get("privacy_filter:unknown"), Some(&3));
    for raw in [
        "alice@example.com",
        "/Users/alice/.ssh/id_rsa",
        "sk-test-raw-token",
    ] {
        assert!(!json.contains(raw), "safe output leaked {raw}");
    }
    assert!(safe.report.warnings.iter().any(|warning| {
        warning == "Privacy Filter sidecar emitted unsupported span label; mapped to unknown."
    }));
}
#[tokio::test]
async fn privacy_filter_sidecar_summary_is_integrated_without_raw_text() {
    let trace = TraceFile {
        model_name: "test-model".to_string(),
        usage: None,
        memory_snapshot: Vec::new(),
        http_exchanges: Vec::new(),
        steps: vec![TraceStep {
            request_hint: None,
            response: TraceResponse::UserInput {
                content: "Alice asked for a project update".to_string(),
            },
            expected_tool_results: Vec::new(),
        }],
    };
    let raw = RawTraceContribution::from_recorded_trace(
        &trace,
        RecordedTraceContributionOptions::default().set_include_message_text(true),
    );
    let envelope = DeterministicTraceRedactor::new(Vec::new())
        .with_privacy_filter(Arc::new(FakePrivacyFilterAdapter))
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

    let json = serde_json::to_string(&envelope).expect("envelope serializes");
    assert!(json.contains(PRIVACY_FILTER_SIDECAR_PIPELINE_SUFFIX));
    assert!(json.contains("<PRIVATE_PERSON_1>"));
    assert!(!json.contains("Alice asked"));
    assert_eq!(
        envelope
            .privacy
            .privacy_filter_summary
            .as_ref()
            .and_then(|summary| summary.by_label.get("private_person"))
            .copied(),
        Some(1)
    );
    assert_eq!(
        envelope
            .privacy
            .redaction_counts
            .get("privacy_filter:private_person"),
        Some(&1)
    );
}
#[tokio::test]
async fn privacy_filter_canary_report_keeps_raw_canary_values_out() {
    let report = run_privacy_filter_canary(&CanaryPrivacyFilterAdapter)
        .await
        .expect("canary should run");
    let json = serde_json::to_string(&report).expect("report serializes");

    assert!(report.healthy);
    assert_eq!(
        report
            .summary
            .as_ref()
            .and_then(|summary| summary.by_label.get("secret")),
        Some(&1)
    );
    for raw_value in synthetic_privacy_filter_canary_values() {
        assert!(!json.contains(&raw_value));
    }
    assert!(json.contains("sha256:"));
    assert!(!json.contains("tc_canary_secret_0123456789abcdef"));
}
#[tokio::test]
async fn privacy_filter_sidecar_failure_falls_back_without_raw_error_text() {
    let trace = TraceFile {
        model_name: "test-model".to_string(),
        usage: None,
        memory_snapshot: Vec::new(),
        http_exchanges: Vec::new(),
        steps: vec![TraceStep {
            request_hint: None,
            response: TraceResponse::UserInput {
                content: "Alice asked for a status update".to_string(),
            },
            expected_tool_results: Vec::new(),
        }],
    };
    let raw = RawTraceContribution::from_recorded_trace(
        &trace,
        RecordedTraceContributionOptions::default().set_include_message_text(true),
    );

    let envelope = DeterministicTraceRedactor::new(Vec::new())
        .with_privacy_filter(Arc::new(FailingPrivacyFilterAdapter))
        .redact_trace(raw)
        .await
        .expect("deterministic fallback should keep redaction non-fatal");
    let json = serde_json::to_string(&envelope).expect("envelope serializes");

    assert!(json.contains("Privacy Filter sidecar failed"));
    assert!(json.contains("sha256:"));
    assert!(!json.contains("tc_canary_secret_0123456789abcdef"));
    assert!(envelope.privacy.privacy_filter_summary.is_none());
    assert!(
        envelope
            .privacy
            .redaction_counts
            .contains_key("privacy_filter:sidecar_failure")
    );
}
#[tokio::test]
async fn command_privacy_filter_error_does_not_echo_stderr() {
    if !Path::new("/bin/sh").exists() {
        return;
    }
    let adapter = CommandPrivacyFilterAdapter::new("/bin/sh").with_args([
        "-c",
        "cat >/dev/null; printf '%s' 'raw-secret-from-stderr' >&2; exit 7",
    ]);

    let error = adapter
        .redact_text("hello")
        .await
        .expect_err("non-zero sidecar exit should fail")
        .to_string();

    assert!(error.contains("stderr_len="));
    assert!(error.contains("stderr_hash="));
    assert!(!error.contains("raw-secret-from-stderr"));
}
#[tokio::test]
async fn command_privacy_filter_adapter_does_not_inherit_trace_commons_tokens() {
    if !Path::new("/bin/sh").exists() {
        return;
    }
    let _env_guard =
        EnvVarRestore::set("TRACE_COMMONS_TENANT_TOKENS", "tenant-a:super-secret-token");

    let adapter = CommandPrivacyFilterAdapter::new("/bin/sh").with_args([
        "-c",
        "cat >/dev/null; printf '{\"redacted_text\":\"%s\"}' \"${TRACE_COMMONS_TENANT_TOKENS-unset}\"",
    ]);
    let redaction = adapter
        .redact_text("hello")
        .await
        .expect("sidecar should run")
        .expect("sidecar should return redaction");

    assert_eq!(redaction.redacted_text, "unset");
}
#[tokio::test]
async fn command_privacy_filter_rejects_oversized_stdout() {
    if !Path::new("/bin/sh").exists() {
        return;
    }
    let adapter = CommandPrivacyFilterAdapter::new("/bin/sh")
        .with_args([
            "-c",
            "cat >/dev/null; printf '%s' '{\"redacted_text\":\"012345678901234567890123456789\"}'",
        ])
        .with_output_limits(16, 16);

    let error = adapter
        .redact_text("hello")
        .await
        .expect_err("oversized stdout should fail")
        .to_string();

    assert!(error.contains("stdout exceeded privacy filter sidecar limit"));
    assert!(!error.contains("0123456789"));
}
#[test]
fn tool_specific_payload_redaction_removes_email_content_fields() {
    let redactor = DeterministicTraceRedactor::new(Vec::new());
    let payload = serde_json::json!({
        "to": ["alice@example.com"],
        "subject": "Project launch",
        "body": "Please review /tmp/ironclaw/private.txt",
        "public_id": "message-1"
    });

    let mut state = RedactionState::default();
    let (redacted, report) = redactor.redact_json_value(Some("gmail_send"), &payload, &mut state);
    let json = serde_json::to_string(&redacted).expect("payload serializes");

    assert!(json.contains("[REDACTED:email_participant]"));
    assert!(json.contains("[REDACTED:email_content]"));
    assert!(json.contains("message-1"));
    assert!(!json.contains("alice@example.com"));
    assert!(!json.contains("Project launch"));
    assert!(!json.contains("/tmp/ironclaw/private.txt"));
    assert_eq!(report.counts.get("tool_sensitive_field"), Some(&3));
}
#[test]
fn tool_specific_payload_redaction_preserves_browser_replay_metadata() {
    let redactor = DeterministicTraceRedactor::new(Vec::new());
    let payload = serde_json::json!({
        "method": "GET",
        "url": "https://example.com/private/customer-123?token=secret-token#frag",
        "headers": {
            "authorization": "Bearer secret-token",
            "accept": "application/json"
        },
        "response": {
            "status": 204,
            "event_id": "evt_public_123"
        }
    });

    let mut state = RedactionState::default();
    let (redacted, report) =
        redactor.redact_json_value(Some("browser_fetch"), &payload, &mut state);
    let json = serde_json::to_string(&redacted).expect("payload serializes");

    assert_eq!(redacted["method"], "GET");
    assert_eq!(redacted["response"]["status"], 204);
    assert_eq!(redacted["response"]["event_id"], "evt_public_123");
    assert!(json.contains("https://example.com/[REDACTED_PATH]"));
    assert!(!json.contains("customer-123"));
    assert!(!json.contains("secret-token"));
    assert!(json.contains("[REDACTED:browser_header]"));
    assert_eq!(report.counts.get("tool_sensitive_field"), Some(&2));
}
#[test]
fn tool_specific_payload_redaction_preserves_issue_tracker_numbers() {
    let redactor = DeterministicTraceRedactor::new(Vec::new());
    let payload = serde_json::json!({
        "issue_number": 42,
        "number": 42,
        "state": "open",
        "status": "triaged",
        "event_id": "evt_issue_public",
        "title": "Customer Acme reported a private failure",
        "body": "Stack trace includes /Users/alice/project/secrets.txt",
        "html_url": "https://github.com/private-org/private-repo/issues/42?auth=secret",
        "assignee": "alice@example.com",
        "repository": "private-org/private-repo"
    });

    let mut state = RedactionState::default();
    let (redacted, report) =
        redactor.redact_json_value(Some("github_issue_create"), &payload, &mut state);
    let json = serde_json::to_string(&redacted).expect("payload serializes");

    assert_eq!(redacted["issue_number"], 42);
    assert_eq!(redacted["number"], 42);
    assert_eq!(redacted["state"], "open");
    assert_eq!(redacted["status"], "triaged");
    assert_eq!(redacted["event_id"], "evt_issue_public");
    assert!(json.contains("https://github.com/[REDACTED_PATH]"));
    assert!(!json.contains("Acme"));
    assert!(!json.contains("alice@example.com"));
    assert!(!json.contains("private-org/private-repo"));
    assert!(!json.contains("/Users/alice/project"));
    assert_eq!(report.counts.get("tool_sensitive_field"), Some(&5));
}
#[test]
fn tool_specific_payload_redaction_covers_calendar_and_messaging_payloads() {
    let redactor = DeterministicTraceRedactor::new(Vec::new());
    let calendar_payload = serde_json::json!({
        "event_id": "evt_calendar_public",
        "status": "confirmed",
        "summary": "Interview with Alice",
        "location": "Alice home office",
        "attendees": [{"email": "alice@example.com"}]
    });
    let slack_payload = serde_json::json!({
        "event_id": "evt_slack_public",
        "ok": true,
        "channel_id": "C123PRIVATE",
        "user_id": "U123PRIVATE",
        "text": "Alice's private launch note"
    });

    let mut state = RedactionState::default();
    let (calendar_redacted, calendar_report) =
        redactor.redact_json_value(Some("calendar_create_event"), &calendar_payload, &mut state);
    let (slack_redacted, slack_report) =
        redactor.redact_json_value(Some("slack_post_message"), &slack_payload, &mut state);
    let json = serde_json::to_string(&(calendar_redacted.clone(), slack_redacted.clone()))
        .expect("payloads serialize");

    assert_eq!(calendar_redacted["event_id"], "evt_calendar_public");
    assert_eq!(calendar_redacted["status"], "confirmed");
    assert_eq!(slack_redacted["event_id"], "evt_slack_public");
    assert_eq!(slack_redacted["ok"], true);
    assert!(json.contains("[REDACTED:calendar_content]"));
    assert!(json.contains("[REDACTED:calendar_participant]"));
    assert!(json.contains("[REDACTED:message_identity]"));
    assert!(json.contains("[REDACTED:message_content]"));
    assert!(!json.contains("Alice"));
    assert!(!json.contains("alice@example.com"));
    assert!(!json.contains("C123PRIVATE"));
    assert_eq!(calendar_report.counts.get("tool_sensitive_field"), Some(&3));
    assert_eq!(slack_report.counts.get("tool_sensitive_field"), Some(&3));
}
#[test]
fn tool_specific_payload_redaction_summarizes_database_rows_and_params() {
    let redactor = DeterministicTraceRedactor::new(Vec::new());
    let payload = serde_json::json!({
        "operation": "select",
        "status_code": 200,
        "query": "select * from customers where email = $1",
        "params": ["alice@example.com"],
        "rows": [
            {"email": "alice@example.com", "token": "secret-token"},
            {"email": "bob@example.com", "token": "other-secret"}
        ]
    });

    let mut state = RedactionState::default();
    let (redacted, report) =
        redactor.redact_json_value(Some("postgres_query"), &payload, &mut state);
    let json = serde_json::to_string(&redacted).expect("payload serializes");

    assert_eq!(redacted["operation"], "select");
    assert_eq!(redacted["status_code"], 200);
    assert_eq!(redacted["params"]["count"], 1);
    assert_eq!(redacted["rows"]["count"], 2);
    assert!(json.contains("[REDACTED:database_content]"));
    assert!(json.contains("[REDACTED:database_query_param]"));
    assert!(json.contains("[REDACTED:database_row]"));
    assert!(!json.contains("alice@example.com"));
    assert!(!json.contains("secret-token"));
    assert!(!json.contains("select * from customers"));
    assert_eq!(report.counts.get("tool_sensitive_field"), Some(&3));
}
