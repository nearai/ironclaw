//! Local submission records, status-sync history, delayed credit, and the credit report and summary.

use std::collections::BTreeMap;

use chrono::Utc;
use uuid::Uuid;

use crate::contribution::*;

#[test]
fn local_trace_records_load_legacy_json_without_history() {
    let scope = format!("trace-local-history-legacy-test-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    let path = trace_records_path(Some(&scope));
    std::fs::create_dir_all(path.parent().expect("trace records path has a parent"))
        .expect("trace records dir exists");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&serde_json::json!([
            {
                "submission_id": submission_id,
                "trace_id": trace_id,
                "endpoint": "https://trace.example.com/v1/traces",
                "status": "submitted",
                "server_status": "accepted",
                "submitted_at": Utc::now(),
                "privacy_risk": "low",
                "redaction_counts": {},
                "credit_points_pending": 1.0,
                "credit_points_final": 1.0,
                "credit_explanation": ["Accepted locally."]
            }
        ]))
        .expect("legacy records serialize"),
    )
    .expect("legacy records write");

    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].submission_id, submission_id);
    assert_eq!(records[0].trace_id, trace_id);
    let serialized = serde_json::to_value(&records[0]).expect("record serializes");
    assert!(serialized.get("history").is_none());

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn status_sync_appends_safe_local_history_event() {
    let scope = format!("trace-local-history-sync-test-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    write_local_trace_records_for_scope(
        Some(&scope),
        &[NodeTraceSubmissionRecord {
            submission_id,
            trace_id,
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 1.0,
            credit_points_final: Some(1.0),
            credit_explanation: vec!["Accepted locally.".to_string()],
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
        }],
    )
    .expect("local record writes");

    let changed = apply_remote_trace_submission_statuses_for_scope(
        Some(&scope),
        &[TraceSubmissionStatusUpdate {
            submission_id,
            trace_id,
            status: "accepted".to_string(),
            credit_points_pending: 1.0,
            credit_points_final: Some(2.0),
            credit_points_ledger: 1.0,
            credit_points_total: Some(2.0),
            explanation: vec!["Accepted after privacy checks.".to_string()],
            delayed_credit_explanations: vec!["Regression coverage bonus: +1.0.".to_string()],
        }],
    )
    .expect("status sync applies");

    assert_eq!(changed, 1);
    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    let serialized = serde_json::to_value(&records[0]).expect("record serializes");
    let history = serialized["history"]
        .as_array()
        .expect("history is present");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["kind"], "status_sync");
    assert_eq!(history[0]["server_status"], "accepted");
    assert_eq!(history[0]["credit_delta"], 1.0);
    assert_eq!(history[0]["delayed_credit_explanation_count"], 1);
    assert!(history[0].get("message").is_none());

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn duplicate_status_sync_does_not_append_duplicate_history() {
    let scope = format!("trace-local-history-duplicate-test-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    write_local_trace_records_for_scope(
        Some(&scope),
        &[NodeTraceSubmissionRecord {
            submission_id,
            trace_id,
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 1.0,
            credit_points_final: Some(1.0),
            credit_explanation: vec!["Accepted locally.".to_string()],
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
        }],
    )
    .expect("local record writes");
    let update = TraceSubmissionStatusUpdate {
        submission_id,
        trace_id,
        status: "accepted".to_string(),
        credit_points_pending: 1.0,
        credit_points_final: Some(2.0),
        credit_points_ledger: 1.0,
        credit_points_total: Some(2.0),
        explanation: vec!["Accepted after privacy checks.".to_string()],
        delayed_credit_explanations: vec!["Regression coverage bonus: +1.0.".to_string()],
    };

    apply_remote_trace_submission_statuses_for_scope(Some(&scope), std::slice::from_ref(&update))
        .expect("first status sync applies");
    apply_remote_trace_submission_statuses_for_scope(Some(&scope), &[update])
        .expect("duplicate status sync applies");

    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    let serialized = serde_json::to_value(&records[0]).expect("record serializes");
    let history = serialized["history"]
        .as_array()
        .expect("history is present");
    assert_eq!(history.len(), 1);
    assert_eq!(records[0].credit_events.len(), 1);

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn local_status_history_does_not_persist_unsafe_remote_fields() {
    let scope = format!("trace-local-history-safety-test-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    write_local_trace_records_for_scope(
        Some(&scope),
        &[NodeTraceSubmissionRecord {
            submission_id,
            trace_id,
            endpoint: Some("https://private.trace.example.com/v1/traces".to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 1.0,
            credit_points_final: Some(1.0),
            credit_explanation: vec!["Accepted locally.".to_string()],
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
        }],
    )
    .expect("local record writes");

    apply_remote_trace_submission_statuses_for_scope(
        Some(&scope),
        &[TraceSubmissionStatusUpdate {
            submission_id,
            trace_id,
            status: "accepted".to_string(),
            credit_points_pending: 1.0,
            credit_points_final: Some(2.0),
            credit_points_ledger: 1.0,
            credit_points_total: Some(2.0),
            explanation: vec![
                "Accepted for alice@example.com under tenant-raw-alpha at https://private.trace.example.com/v1/traces".to_string(),
            ],
            delayed_credit_explanations: vec![
                "Read /Users/alice/private/token.txt with sk-test-raw-token-123456789".to_string(),
            ],
        }],
    )
    .expect("status sync applies");

    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    let safe_local_credit_projection = serde_json::json!({
        "credit_explanation": &records[0].credit_explanation,
        "credit_events": &records[0].credit_events,
        "history": &records[0].history,
    });
    let serialized =
        serde_json::to_string(&safe_local_credit_projection).expect("records serialize");
    assert!(!serialized.contains("alice@example.com"));
    assert!(!serialized.contains("tenant-raw-alpha"));
    assert!(!serialized.contains("https://private.trace.example.com"));
    assert!(!serialized.contains("/Users/alice/private"));
    assert!(!serialized.contains("sk-test-raw-token"));

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn delayed_credit_sync_resets_notice_and_notice_marks_records() {
    let scope = format!("trace-credit-sync-test-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    write_trace_policy_for_scope(
        Some(&scope),
        &StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
            .set_credit_notice_interval_hours(168),
    )
    .expect("policy writes");
    write_local_trace_records_for_scope(
        Some(&scope),
        &[NodeTraceSubmissionRecord {
            submission_id,
            trace_id,
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 1.0,
            credit_points_final: None,
            credit_explanation: vec!["Accepted locally.".to_string()],
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: Some(Utc::now()),
            credit_notice_state: TraceCreditNoticeState::default(),
        }],
    )
    .expect("local record writes");

    let changed = apply_remote_trace_submission_statuses_for_scope(
        Some(&scope),
        &[TraceSubmissionStatusUpdate {
            submission_id,
            trace_id,
            status: "accepted".to_string(),
            credit_points_pending: 1.0,
            credit_points_final: Some(2.0),
            credit_points_ledger: 1.0,
            credit_points_total: Some(2.0),
            explanation: vec!["Accepted after privacy checks.".to_string()],
            delayed_credit_explanations: vec!["Regression coverage bonus: +1.0.".to_string()],
        }],
    )
    .expect("status sync applies");
    assert_eq!(changed, 1);

    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    assert_eq!(records[0].credit_points_final, Some(2.0));
    assert!(records[0].last_credit_notice_at.is_none());
    assert_eq!(records[0].credit_events.len(), 1);

    let notice = mark_trace_credit_notice_due_for_scope(Some(&scope))
        .expect("notice check succeeds")
        .expect("notice should be due after changed credit");
    assert_eq!(notice.pending_credit, 1.0);
    assert_eq!(notice.final_credit, 2.0);
    assert_eq!(notice.delayed_credit_delta, 1.0);
    assert_eq!(notice.credit_events_total, 1);
    assert!(
        notice
            .recent_explanations
            .iter()
            .any(|reason| reason.contains("Regression coverage bonus"))
    );

    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    assert!(records[0].last_credit_notice_at.is_some());
}
#[test]
fn delayed_credit_explanation_change_resets_notice_without_net_credit_delta() {
    let scope = format!("trace-credit-explanation-test-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    write_local_trace_records_for_scope(
        Some(&scope),
        &[NodeTraceSubmissionRecord {
            submission_id,
            trace_id,
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 1.0,
            credit_points_final: Some(2.0),
            credit_explanation: vec!["Previous credit explanation.".to_string()],
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: Some(Utc::now()),
            credit_notice_state: TraceCreditNoticeState::default(),
        }],
    )
    .expect("local record writes");

    let changed = apply_remote_trace_submission_statuses_for_scope(
        Some(&scope),
        &[TraceSubmissionStatusUpdate {
            submission_id,
            trace_id,
            status: "accepted".to_string(),
            credit_points_pending: 1.0,
            credit_points_final: Some(2.0),
            credit_points_ledger: 1.0,
            credit_points_total: Some(2.0),
            explanation: vec!["Accepted after privacy checks.".to_string()],
            delayed_credit_explanations: vec![
                "Process evaluation utility credited without changing total.".to_string(),
            ],
        }],
    )
    .expect("status sync applies");
    assert_eq!(changed, 1);

    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    assert!(records[0].last_credit_notice_at.is_none());
    assert_eq!(records[0].credit_events.len(), 1);
    assert_eq!(records[0].credit_events[0].points_delta, 0.0);
    assert!(
        records[0]
            .credit_explanation
            .iter()
            .any(|explanation| { explanation.contains("Process evaluation utility credited") })
    );
}
#[test]
fn revoked_credit_change_still_produces_a_notice() {
    let scope = format!("trace-credit-revoked-test-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    write_local_trace_records_for_scope(
        Some(&scope),
        &[NodeTraceSubmissionRecord {
            submission_id,
            trace_id,
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 1.0,
            credit_points_final: Some(1.0),
            credit_explanation: vec!["Accepted locally.".to_string()],
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: Some(Utc::now()),
            credit_notice_state: TraceCreditNoticeState::default(),
        }],
    )
    .expect("local record writes");

    apply_remote_trace_submission_statuses_for_scope(
        Some(&scope),
        &[TraceSubmissionStatusUpdate {
            submission_id,
            trace_id,
            status: "revoked".to_string(),
            credit_points_pending: 0.0,
            credit_points_final: Some(0.0),
            credit_points_ledger: 0.0,
            credit_points_total: Some(0.0),
            explanation: vec!["Submission revoked.".to_string()],
            delayed_credit_explanations: Vec::new(),
        }],
    )
    .expect("status sync applies");

    let notice = mark_trace_credit_noticed_if_due(Some(&scope), 168)
        .expect("notice check succeeds")
        .expect("revoked credit delta should still be noticeable");
    assert_eq!(notice.submissions_revoked, 1);
    assert_eq!(notice.final_credit, 0.0);
    assert!(
        notice
            .recent_explanations
            .iter()
            .any(|reason| reason.contains("Submission revoked"))
    );
}
#[test]
fn expired_status_sync_stops_resubmission_and_reports_expired_credit() {
    let scope = format!("trace-credit-expired-test-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    write_local_trace_records_for_scope(
        Some(&scope),
        &[NodeTraceSubmissionRecord {
            submission_id,
            trace_id,
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 1.0,
            credit_points_final: Some(1.0),
            credit_explanation: vec!["Accepted locally.".to_string()],
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: Some(Utc::now()),
            credit_notice_state: TraceCreditNoticeState::default(),
        }],
    )
    .expect("local record writes");

    apply_remote_trace_submission_statuses_for_scope(
        Some(&scope),
        &[TraceSubmissionStatusUpdate {
            submission_id,
            trace_id,
            status: "expired".to_string(),
            credit_points_pending: 1.0,
            credit_points_final: Some(1.0),
            credit_points_ledger: 0.0,
            credit_points_total: Some(1.0),
            explanation: vec!["Expired under retention policy.".to_string()],
            delayed_credit_explanations: Vec::new(),
        }],
    )
    .expect("status sync applies");

    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    assert_eq!(records[0].status, NodeTraceSubmissionStatus::Expired);
    assert_eq!(trace_credit_summary(&records).submissions_expired, 1);
    assert!(records[0].last_credit_notice_at.is_none());
}
#[test]
fn trace_credit_report_groups_remote_status_and_delayed_credit_events() {
    let submitted_at = Utc::now();
    let accepted_id = Uuid::new_v4();
    let quarantined_id = Uuid::new_v4();
    let rejected_id = Uuid::new_v4();
    let sync_event_at = submitted_at + chrono::Duration::minutes(5);
    let records = vec![
        NodeTraceSubmissionRecord {
            submission_id: accepted_id,
            trace_id: Uuid::new_v4(),
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(submitted_at),
            revoked_at: None,
            privacy_risk: "Low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 2.0,
            credit_points_final: Some(3.5),
            credit_explanation: vec![
                "Accepted after privacy checks.".to_string(),
                "Regression coverage bonus: +1.5.".to_string(),
            ],
            credit_events: vec![
                TraceCreditEvent {
                    event_id: Uuid::new_v4(),
                    submission_id: accepted_id,
                    contributor_pseudonym: "local".to_string(),
                    kind: TraceCreditEventKind::Accepted,
                    points_delta: 2.0,
                    reason: "Accepted for private Trace Commons processing.".to_string(),
                    created_at: submitted_at,
                },
                TraceCreditEvent {
                    event_id: Uuid::new_v4(),
                    submission_id: accepted_id,
                    contributor_pseudonym: "local-sync".to_string(),
                    kind: TraceCreditEventKind::CreditSynced,
                    points_delta: 1.5,
                    reason: "Server status synced as accepted; delayed ledger credit now +1.50."
                        .to_string(),
                    created_at: sync_event_at,
                },
            ],
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
        },
        NodeTraceSubmissionRecord {
            submission_id: quarantined_id,
            trace_id: Uuid::new_v4(),
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some("quarantined".to_string()),
            submitted_at: Some(submitted_at + chrono::Duration::minutes(2)),
            revoked_at: None,
            privacy_risk: "Medium".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 0.0,
            credit_points_final: None,
            credit_explanation: vec![
                "Submission is quarantined until privacy review completes.".to_string(),
            ],
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
        },
        NodeTraceSubmissionRecord {
            submission_id: rejected_id,
            trace_id: Uuid::new_v4(),
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some("rejected".to_string()),
            submitted_at: Some(submitted_at + chrono::Duration::minutes(1)),
            revoked_at: None,
            privacy_risk: "High".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 0.0,
            credit_points_final: Some(0.0),
            credit_explanation: vec!["Rejected during privacy review.".to_string()],
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
        },
    ];

    let report = trace_credit_report(&records);

    assert_eq!(report.submissions_total, 3);
    assert_eq!(report.submissions_submitted, 3);
    assert_eq!(report.submissions_accepted, 1);
    assert_eq!(report.submissions_quarantined, 1);
    assert_eq!(report.submissions_rejected, 1);
    assert_eq!(report.pending_credit, 2.0);
    assert_eq!(report.final_credit, 3.5);
    assert_eq!(report.credit_events_total, 2);
    assert_eq!(report.delayed_credit_delta, 1.5);
    assert_eq!(
        report.last_submission_at,
        Some(submitted_at + chrono::Duration::minutes(2))
    );
    assert_eq!(report.last_credit_sync_at, Some(sync_event_at));
    assert!(
        report
            .explanation_lines
            .iter()
            .any(|line| line.contains("1 accepted"))
    );
    assert!(
        report
            .explanation_lines
            .iter()
            .any(|line| line.contains("1 quarantined"))
    );
    assert!(
        report
            .explanation_lines
            .iter()
            .any(|line| line.contains("1 rejected"))
    );
    assert!(
        report
            .explanation_lines
            .iter()
            .any(|line| line.contains("Regression coverage bonus"))
    );
}
#[test]
fn trace_credit_summary_uses_richer_report_totals_without_changing_shape() {
    let record = NodeTraceSubmissionRecord {
        submission_id: Uuid::new_v4(),
        trace_id: Uuid::new_v4(),
        endpoint: Some("https://trace.example.com/v1/traces".to_string()),
        status: NodeTraceSubmissionStatus::Purged,
        server_status: Some("expired".to_string()),
        submitted_at: Some(Utc::now()),
        revoked_at: None,
        privacy_risk: "Low".to_string(),
        redaction_counts: BTreeMap::new(),
        credit_points_pending: 4.0,
        credit_points_final: Some(4.0),
        credit_explanation: vec!["Expired under retention policy.".to_string()],
        credit_events: Vec::new(),
        history: Vec::new(),
        last_credit_notice_at: None,
        credit_notice_state: TraceCreditNoticeState::default(),
    };

    let summary = trace_credit_summary(&[record]);

    assert_eq!(summary.submissions_total, 1);
    assert_eq!(summary.submissions_expired, 1);
    assert_eq!(summary.pending_credit, 4.0);
    assert_eq!(summary.final_credit, 4.0);
    assert_eq!(
        summary.recent_explanations,
        vec!["Expired under retention policy.".to_string()]
    );
}
