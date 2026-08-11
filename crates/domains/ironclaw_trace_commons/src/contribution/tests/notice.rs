//! Scoped credit views, queue diagnostics, and the credit-notice state machine and outbox.

use std::time::Duration;

use chrono::Utc;
use uuid::Uuid;

use crate::contribution::*;

use super::support::*;

#[tokio::test]
async fn trace_scope_flushes_serialize_same_scope_without_blocking_other_scopes() {
    let scope = format!("trace-lock-test-{}", Uuid::new_v4());
    let other_scope = format!("trace-lock-other-test-{}", Uuid::new_v4());
    let first_guard = lock_trace_scope_for_mutation(Some(&scope)).await;

    let same_scope = scope.clone();
    let mut same_scope_waiter = Box::pin(tokio::spawn(async move {
        flush_trace_contribution_queue_for_scope(Some(&same_scope), 1).await
    }));

    let other_scope_waiter = tokio::spawn(async move {
        flush_trace_contribution_queue_for_scope(Some(&other_scope), 1).await
    });

    let other_scope_result = tokio::time::timeout(Duration::from_millis(200), other_scope_waiter)
        .await
        .expect("different scope should not be blocked")
        .expect("different scope waiter should complete");
    assert!(
        other_scope_result
            .expect_err("default disabled policy should make flush exit")
            .to_string()
            .contains("opt-in is disabled")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), same_scope_waiter.as_mut())
            .await
            .is_err(),
        "same scope waiter should remain serialized behind the first guard"
    );

    drop(first_guard);
    let same_scope_result =
        tokio::time::timeout(Duration::from_millis(200), same_scope_waiter.as_mut())
            .await
            .expect("same scope waiter should complete after the first guard is dropped")
            .expect("same scope waiter should not panic");
    assert!(
        same_scope_result
            .expect_err("default disabled policy should make flush exit")
            .to_string()
            .contains("opt-in is disabled")
    );
}
#[test]
fn status_sync_endpoint_is_derived_from_submission_endpoint() {
    assert_eq!(
        trace_submission_status_endpoint("https://trace.example.com/v1/traces")
            .expect("endpoint parses"),
        "https://trace.example.com/v1/contributors/me/submission-status"
    );
    assert_eq!(
        trace_submission_status_endpoint("https://trace.example.com/api/v1/traces?x=1")
            .expect("endpoint parses"),
        "https://trace.example.com/api/v1/contributors/me/submission-status"
    );
}
#[test]
fn scoped_credit_view_reflects_record_changes_via_signature() {
    let scope = format!("scoped-credit-view-{}", Uuid::new_v4());

    // No records yet -> zero report.
    let view = scoped_credit_view(&scope).expect("empty view");
    assert_eq!(view.report.submissions_total, 0);
    assert!(view.manual_review_holds.is_empty());

    // One submitted record -> view reflects it (cache miss, recompute).
    write_local_trace_records_for_scope(
        Some(&scope),
        &[submitted_credit_record(1.0, None, None, Vec::new())],
    )
    .expect("write one record");
    let view = scoped_credit_view(&scope).expect("one-record view");
    assert_eq!(view.report.submissions_total, 1);
    assert_eq!(view.report.submissions_submitted, 1);

    // A repeated call with no change returns the same view (cache hit path).
    assert_eq!(
        scoped_credit_view(&scope).expect("cache-hit view").report,
        view.report
    );

    // Changing the records file changes its signature -> the cached view is
    // invalidated and recomputed, so the new total is reflected (a stale
    // cache would still report 1).
    write_local_trace_records_for_scope(
        Some(&scope),
        &[
            submitted_credit_record(1.0, None, None, Vec::new()),
            submitted_credit_record(2.0, None, None, Vec::new()),
        ],
    )
    .expect("write two records");
    let view = scoped_credit_view(&scope).expect("two-record view");
    assert_eq!(view.report.submissions_total, 2);

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn queue_diagnostics_are_scoped_to_one_user_queue_and_records() {
    let scope_a = format!("trace-queue-diagnostics-a-{}", Uuid::new_v4());
    let scope_b = format!("trace-queue-diagnostics-b-{}", Uuid::new_v4());
    let policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string());
    write_trace_policy_for_scope(Some(&scope_a), &policy).expect("scope a policy writes");
    write_trace_policy_for_scope(Some(&scope_b), &policy).expect("scope b policy writes");

    let queue_a = trace_queue_dir(Some(&scope_a));
    let queue_b = trace_queue_dir(Some(&scope_b));
    std::fs::create_dir_all(&queue_a).expect("scope a queue exists");
    std::fs::create_dir_all(&queue_b).expect("scope b queue exists");
    std::fs::write(queue_a.join(format!("{}.json", Uuid::new_v4())), "{}")
        .expect("scope a queued fixture writes");
    std::fs::write(queue_b.join(format!("{}.json", Uuid::new_v4())), "{}")
        .expect("scope b queued fixture writes");

    let sync_at = Utc::now();
    let mut scope_a_record = submitted_credit_record(
        1.0,
        Some(1.5),
        None,
        vec!["Accepted for scope a.".to_string()],
    );
    scope_a_record.credit_events.push(TraceCreditEvent {
        event_id: Uuid::new_v4(),
        submission_id: scope_a_record.submission_id,
        contributor_pseudonym: "local-sync".to_string(),
        kind: TraceCreditEventKind::CreditSynced,
        points_delta: 0.5,
        reason: "Server status synced as accepted.".to_string(),
        created_at: sync_at,
    });
    write_local_trace_records_for_scope(Some(&scope_a), &[scope_a_record])
        .expect("scope a records write");
    write_local_trace_records_for_scope(
        Some(&scope_b),
        &[NodeTraceSubmissionRecord {
            status: NodeTraceSubmissionStatus::Revoked,
            revoked_at: Some(Utc::now()),
            ..submitted_credit_record(
                0.0,
                Some(0.0),
                None,
                vec!["Revoked for scope b.".to_string()],
            )
        }],
    )
    .expect("scope b records write");

    let diagnostics_a =
        trace_queue_diagnostics_for_scope(Some(&scope_a)).expect("scope a diagnostics read");
    let diagnostics_b =
        trace_queue_diagnostics_for_scope(Some(&scope_b)).expect("scope b diagnostics read");

    assert_eq!(diagnostics_a.queued_count, 1);
    assert_eq!(diagnostics_a.submitted_count, 1);
    assert_eq!(diagnostics_a.revoked_count, 0);
    assert!(diagnostics_a.policy_enabled);
    assert!(diagnostics_a.endpoint_configured);
    assert!(diagnostics_a.ready_to_flush);
    assert!(diagnostics_a.last_submission_at.is_some());
    assert_eq!(diagnostics_a.last_credit_sync_at, Some(sync_at));

    assert_eq!(diagnostics_b.queued_count, 1);
    assert_eq!(diagnostics_b.submitted_count, 0);
    assert_eq!(diagnostics_b.revoked_count, 1);
    assert!(diagnostics_b.ready_to_flush);

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope_a)));
    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope_b)));
}
#[test]
fn queue_diagnostics_aggregates_sanitized_hold_reasons() {
    let scope = format!("trace-queue-diagnostics-holds-{}", Uuid::new_v4());
    let policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string());
    write_trace_policy_for_scope(Some(&scope), &policy).expect("policy writes");
    let dir = trace_queue_dir(Some(&scope));
    std::fs::create_dir_all(&dir).expect("queue dir exists");

    let raw_reason =
        "manual review for alice@example.com in /Users/alice/private with sk-test-raw-token";
    for _ in 0..2 {
        let queue_path = dir.join(format!("{}.json", Uuid::new_v4()));
        std::fs::write(&queue_path, "{}").expect("queued fixture writes");
        write_trace_queue_hold_reason(&queue_path, raw_reason).expect("hold reason writes");
    }

    let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics read");

    assert_eq!(diagnostics.queued_count, 2);
    assert_eq!(diagnostics.held_count, 2);
    assert_eq!(
        diagnostics
            .held_reason_counts
            .values()
            .copied()
            .sum::<u32>(),
        2
    );
    assert_eq!(diagnostics.held_reason_counts.len(), 1);
    let aggregated_reason = diagnostics
        .held_reason_counts
        .keys()
        .next()
        .expect("held reason is present");
    assert!(!aggregated_reason.contains("alice@example.com"));
    assert!(!aggregated_reason.contains("/Users/alice/private"));
    assert!(!aggregated_reason.contains("sk-test-raw-token"));

    let serialized = serde_json::to_string(&diagnostics).expect("diagnostics serialize");
    assert!(!serialized.contains("alice@example.com"));
    assert!(!serialized.contains("/Users/alice/private"));
    assert!(!serialized.contains("sk-test-raw-token"));

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn credit_notice_snapshot_returns_none_when_policy_disabled_or_interval_zero() {
    let disabled_scope = format!("trace-credit-disabled-notice-test-{}", Uuid::new_v4());
    write_local_trace_records_for_scope(
        Some(&disabled_scope),
        &[submitted_credit_record(
            1.0,
            Some(1.0),
            None,
            vec!["Accepted locally.".to_string()],
        )],
    )
    .expect("disabled scope record writes");

    let disabled_notice = mark_trace_credit_notice_due_for_scope(Some(&disabled_scope))
        .expect("disabled notice check succeeds");
    assert_eq!(disabled_notice, None);
    let disabled_records =
        read_local_trace_records_for_scope(Some(&disabled_scope)).expect("records read");
    assert!(
        disabled_records[0].last_credit_notice_at.is_none(),
        "disabled policy must not mark the local notice as seen"
    );

    let zero_interval_scope = format!("trace-credit-zero-interval-notice-test-{}", Uuid::new_v4());
    write_trace_policy_for_scope(
        Some(&zero_interval_scope),
        &StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
            .set_credit_notice_interval_hours(0),
    )
    .expect("zero interval policy writes");
    write_local_trace_records_for_scope(
        Some(&zero_interval_scope),
        &[submitted_credit_record(
            2.0,
            Some(2.5),
            None,
            vec!["Delayed utility credit posted.".to_string()],
        )],
    )
    .expect("zero interval scope record writes");

    let zero_interval_notice = mark_trace_credit_notice_due_for_scope(Some(&zero_interval_scope))
        .expect("zero interval notice check succeeds");
    assert_eq!(zero_interval_notice, None);
    let zero_interval_records =
        read_local_trace_records_for_scope(Some(&zero_interval_scope)).expect("records read");
    assert!(
        zero_interval_records[0].last_credit_notice_at.is_none(),
        "zero interval policy must leave the notice unmarked"
    );
}
#[test]
fn scoped_credit_notice_snapshot_marks_only_that_scope() {
    let due_scope = format!("trace-credit-due-scope-test-{}", Uuid::new_v4());
    let untouched_scope = format!("trace-credit-untouched-scope-test-{}", Uuid::new_v4());
    let policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
        .set_credit_notice_interval_hours(168);
    write_trace_policy_for_scope(Some(&due_scope), &policy).expect("due policy writes");
    write_trace_policy_for_scope(Some(&untouched_scope), &policy).expect("untouched policy writes");
    write_local_trace_records_for_scope(
        Some(&due_scope),
        &[submitted_credit_record(
            1.5,
            Some(2.0),
            None,
            vec!["Accepted after privacy checks.".to_string()],
        )],
    )
    .expect("due record writes");
    write_local_trace_records_for_scope(
        Some(&untouched_scope),
        &[submitted_credit_record(
            9.0,
            Some(10.0),
            None,
            vec!["Should not be marked by another scope.".to_string()],
        )],
    )
    .expect("untouched record writes");

    let notice = mark_trace_credit_notice_due_for_scope(Some(&due_scope))
        .expect("scoped notice check succeeds")
        .expect("due scope should produce a notice");

    assert_eq!(notice.submissions_submitted, 1);
    assert_eq!(notice.pending_credit, 1.5);
    assert_eq!(notice.final_credit, 2.0);

    let due_records = read_local_trace_records_for_scope(Some(&due_scope)).expect("records");
    assert!(due_records[0].last_credit_notice_at.is_some());
    let untouched_records =
        read_local_trace_records_for_scope(Some(&untouched_scope)).expect("records");
    assert!(
        untouched_records[0].last_credit_notice_at.is_none(),
        "checking one scope must not mark another scope's local credit notice"
    );
}
#[test]
fn credit_notice_acknowledge_suppresses_same_fingerprint_until_credit_changes() {
    let scope = format!("trace-credit-ack-test-{}", Uuid::new_v4());
    write_trace_policy_for_scope(
        Some(&scope),
        &StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
            .set_credit_notice_interval_hours(168),
    )
    .expect("policy writes");
    let record = submitted_credit_record(
        1.0,
        Some(1.0),
        None,
        vec!["Accepted after privacy checks.".to_string()],
    );
    let submission_id = record.submission_id;
    let trace_id = record.trace_id;
    write_local_trace_records_for_scope(Some(&scope), &[record]).expect("record writes");

    let due = trace_credit_notice_due_for_scope(Some(&scope))
        .expect("notice due check succeeds")
        .expect("notice starts due");
    assert_eq!(due.final_credit, 1.0);
    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    assert!(records[0].last_credit_notice_at.is_none());
    assert!(records[0].credit_notice_state.is_empty());

    let acknowledged = acknowledge_trace_credit_notice_for_scope(Some(&scope))
        .expect("acknowledge succeeds")
        .expect("acknowledge returns the current summary");
    assert_eq!(acknowledged.final_credit, 1.0);

    let after_ack =
        trace_credit_notice_due_for_scope(Some(&scope)).expect("notice due check succeeds");
    assert_eq!(after_ack, None);
    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    assert!(records[0].credit_notice_state.acknowledged_at.is_some());

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
            delayed_credit_explanations: vec!["Benchmark conversion bonus: +1.0.".to_string()],
        }],
    )
    .expect("status sync applies");
    assert_eq!(changed, 1);

    let after_change = trace_credit_notice_due_for_scope(Some(&scope))
        .expect("notice due check succeeds")
        .expect("changed credit is due again");
    assert_eq!(after_change.final_credit, 2.0);
    assert_eq!(after_change.delayed_credit_delta, 1.0);

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn credit_notice_snooze_suppresses_until_deadline() {
    let scope = format!("trace-credit-snooze-test-{}", Uuid::new_v4());
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
        &[submitted_credit_record(
            1.0,
            Some(1.5),
            None,
            vec!["Delayed utility credit posted.".to_string()],
        )],
    )
    .expect("record writes");
    let now = Utc::now();
    let snoozed_until = now + chrono::Duration::hours(24);

    assert!(
        trace_credit_notice_due_for_scope_at(Some(&scope), now)
            .expect("notice due check succeeds")
            .is_some()
    );
    let snoozed = snooze_trace_credit_notice_for_scope_until_at(Some(&scope), snoozed_until, now)
        .expect("snooze succeeds")
        .expect("snooze returns the current summary");
    assert_eq!(snoozed.final_credit, 1.5);

    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    assert_eq!(
        records[0].credit_notice_state.snoozed_until,
        Some(snoozed_until)
    );
    assert_eq!(
        trace_credit_notice_due_for_scope_at(Some(&scope), now + chrono::Duration::hours(1))
            .expect("notice due check succeeds"),
        None
    );
    assert!(
        trace_credit_notice_due_for_scope_at(Some(&scope), now + chrono::Duration::hours(25))
            .expect("notice due check succeeds")
            .is_some()
    );

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn legacy_credit_notice_timestamp_suppresses_until_interval() {
    let scope = format!("trace-credit-legacy-notice-test-{}", Uuid::new_v4());
    write_trace_policy_for_scope(
        Some(&scope),
        &StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
            .set_credit_notice_interval_hours(168),
    )
    .expect("policy writes");
    let now = Utc::now();
    write_local_trace_records_for_scope(
        Some(&scope),
        &[submitted_credit_record(
            1.0,
            Some(1.5),
            Some(now),
            vec!["Previously noticed before the state field existed.".to_string()],
        )],
    )
    .expect("record writes");

    assert_eq!(
        trace_credit_notice_due_for_scope_at(Some(&scope), now + chrono::Duration::hours(1))
            .expect("notice due check succeeds"),
        None
    );
    assert!(
        trace_credit_notice_due_for_scope_at(Some(&scope), now + chrono::Duration::hours(169))
            .expect("notice due check succeeds")
            .is_some()
    );

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn trace_credit_notice_outbox_enqueue_is_idempotent_per_fingerprint() {
    let scope = format!("trace-credit-outbox-idempotent-test-{}", Uuid::new_v4());
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
        &[submitted_credit_record(
            1.0,
            Some(1.5),
            None,
            vec!["Delayed utility credit posted.".to_string()],
        )],
    )
    .expect("record writes");

    let now = Utc::now();
    let first = mark_trace_credit_noticed_if_due_at_unlocked(Some(&scope), 168, now)
        .expect("first notice check succeeds")
        .expect("first notice is due");
    assert_eq!(first.final_credit, 1.5);
    let second = mark_trace_credit_noticed_if_due_at_unlocked(
        Some(&scope),
        168,
        now + chrono::Duration::hours(169),
    )
    .expect("second notice check succeeds")
    .expect("same fingerprint is due again after interval");
    assert_eq!(second.final_credit, 1.5);

    let outbox = read_trace_credit_notice_outbox_for_scope(Some(&scope))
        .expect("credit notice outbox reads");
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].status, TraceCreditNoticeOutboxStatus::Pending);
    assert_eq!(outbox[0].attempt_count, 0);
    assert!(outbox[0].message.contains("pending +1.00"));

    let pending = pending_trace_credit_notice_outbox_items_for_scope_at(Some(&scope), now)
        .expect("pending outbox reads");
    assert_eq!(pending.len(), 1);

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn credit_notice_delivery_success_marks_outbox_delivered() {
    let scope = format!("trace-credit-outbox-delivered-test-{}", Uuid::new_v4());
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
        &[submitted_credit_record(
            2.0,
            Some(3.0),
            None,
            vec!["Benchmark conversion bonus posted.".to_string()],
        )],
    )
    .expect("record writes");
    mark_trace_credit_noticed_if_due_at_unlocked(Some(&scope), 168, Utc::now())
        .expect("notice check succeeds")
        .expect("notice is due");
    let fingerprint = read_trace_credit_notice_outbox_for_scope(Some(&scope))
        .expect("outbox reads")[0]
        .fingerprint
        .clone();

    let delivered =
        record_trace_credit_notice_delivery_success_for_scope(Some(&scope), &fingerprint, "test")
            .expect("delivery success records")
            .expect("outbox item exists");

    assert_eq!(delivered.status, TraceCreditNoticeOutboxStatus::Delivered);
    assert_eq!(delivered.attempt_count, 1);
    assert!(delivered.delivered_at.is_some());
    assert_eq!(delivered.delivery_attempts.len(), 1);
    assert!(delivered.delivery_attempts[0].succeeded);
    assert!(
        pending_trace_credit_notice_outbox_items_for_scope_at(Some(&scope), Utc::now())
            .expect("pending outbox reads")
            .is_empty()
    );

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn credit_notice_delivery_failure_keeps_pending_with_safe_error_hash() {
    let scope = format!("trace-credit-outbox-failure-test-{}", Uuid::new_v4());
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
        &[submitted_credit_record(
            4.0,
            Some(5.0),
            None,
            vec!["Regression catch bonus posted.".to_string()],
        )],
    )
    .expect("record writes");
    let now = Utc::now();
    mark_trace_credit_noticed_if_due_at_unlocked(Some(&scope), 168, now)
        .expect("notice check succeeds")
        .expect("notice is due");
    let fingerprint = read_trace_credit_notice_outbox_for_scope(Some(&scope))
        .expect("outbox reads")[0]
        .fingerprint
        .clone();

    let failed = record_trace_credit_notice_delivery_failure_for_scope(
        Some(&scope),
        &fingerprint,
        "test",
        "failed for alice@example.com using sk-test-secret in /Users/alice/private",
    )
    .expect("delivery failure records")
    .expect("outbox item exists");

    assert_eq!(failed.status, TraceCreditNoticeOutboxStatus::Pending);
    assert_eq!(failed.attempt_count, 1);
    assert!(failed.next_attempt_at.is_some());
    assert_eq!(failed.delivery_attempts.len(), 1);
    let attempt = &failed.delivery_attempts[0];
    assert!(!attempt.succeeded);
    assert_eq!(attempt.channel, "test");
    assert!(
        attempt
            .error_hash
            .as_deref()
            .is_some_and(|hash| hash.starts_with("sha256:"))
    );
    assert!(attempt.error_kind.is_some());
    let serialized = serde_json::to_string(&failed).expect("outbox serializes");
    assert!(!serialized.contains("alice@example.com"));
    assert!(!serialized.contains("sk-test-secret"));
    assert!(!serialized.contains("/Users/alice/private"));

    assert!(
        pending_trace_credit_notice_outbox_items_for_scope_at(Some(&scope), now)
            .expect("pending before retry reads")
            .is_empty(),
        "failed delivery should wait until next_attempt_at before retry"
    );
    assert_eq!(
        pending_trace_credit_notice_outbox_items_for_scope_at(
            Some(&scope),
            failed.next_attempt_at.expect("next attempt exists")
        )
        .expect("pending after retry reads")
        .len(),
        1
    );

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn credit_notice_acknowledge_and_snooze_suppress_pending_outbox_items() {
    let ack_scope = format!("trace-credit-outbox-ack-test-{}", Uuid::new_v4());
    let snooze_scope = format!("trace-credit-outbox-snooze-test-{}", Uuid::new_v4());
    for scope in [&ack_scope, &snooze_scope] {
        write_trace_policy_for_scope(
            Some(scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
                .set_credit_notice_interval_hours(168),
        )
        .expect("policy writes");
        write_local_trace_records_for_scope(
            Some(scope),
            &[submitted_credit_record(
                1.0,
                Some(2.0),
                None,
                vec!["Delayed utility credit posted.".to_string()],
            )],
        )
        .expect("record writes");
        mark_trace_credit_noticed_if_due_at_unlocked(Some(scope), 168, Utc::now())
            .expect("notice check succeeds")
            .expect("notice is due");
    }

    acknowledge_trace_credit_notice_for_scope_at_unlocked(Some(&ack_scope), Utc::now())
        .expect("ack succeeds")
        .expect("ack returns summary");
    let ack_outbox =
        read_trace_credit_notice_outbox_for_scope(Some(&ack_scope)).expect("outbox reads");
    assert_eq!(
        ack_outbox[0].status,
        TraceCreditNoticeOutboxStatus::Acknowledged
    );
    assert!(
        pending_trace_credit_notice_outbox_items_for_scope_at(Some(&ack_scope), Utc::now())
            .expect("pending ack outbox reads")
            .is_empty()
    );

    let now = Utc::now();
    let snoozed_until = now + chrono::Duration::hours(4);
    snooze_trace_credit_notice_for_scope_until_at_unlocked(Some(&snooze_scope), snoozed_until, now)
        .expect("snooze succeeds")
        .expect("snooze returns summary");
    let snooze_outbox =
        read_trace_credit_notice_outbox_for_scope(Some(&snooze_scope)).expect("outbox reads");
    assert_eq!(
        snooze_outbox[0].status,
        TraceCreditNoticeOutboxStatus::Snoozed
    );
    assert_eq!(snooze_outbox[0].snoozed_until, Some(snoozed_until));
    assert!(
        pending_trace_credit_notice_outbox_items_for_scope_at(
            Some(&snooze_scope),
            now + chrono::Duration::hours(1)
        )
        .expect("pending snoozed outbox reads")
        .is_empty()
    );
    assert_eq!(
        pending_trace_credit_notice_outbox_items_for_scope_at(Some(&snooze_scope), snoozed_until)
            .expect("pending after snooze reads")
            .len(),
        1
    );

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&ack_scope)));
    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&snooze_scope)));
}
