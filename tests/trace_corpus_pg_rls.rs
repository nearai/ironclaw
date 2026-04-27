#![cfg(feature = "postgres")]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use ironclaw::config::{DatabaseBackend, DatabaseConfig, SslMode};
use ironclaw::db::{Database, TraceCorpusRlsDiagnostics, postgres::PgBackend};
use ironclaw::trace_corpus_storage::{
    TenantScopedTraceObjectRef, TraceAuditAction, TraceAuditEventWrite, TraceAuditSafeMetadata,
    TraceCorpusStatus, TraceCorpusStore, TraceCreditEventType, TraceCreditEventWrite,
    TraceCreditSettlementState, TraceDerivedRecordWrite, TraceDerivedStatus,
    TraceExportManifestItemInvalidationReason, TraceExportManifestItemWrite,
    TraceExportManifestWrite, TraceObjectArtifactKind, TraceObjectRefWrite,
    TraceRetentionJobItemAction, TraceRetentionJobItemStatus, TraceRetentionJobItemWrite,
    TraceRetentionJobStatus, TraceRetentionJobWrite, TraceSubmissionWrite, TraceTombstoneWrite,
    TraceVectorEntrySourceProjection, TraceVectorEntryStatus, TraceVectorEntryWrite,
    TraceWorkerKind,
};
use secrecy::{ExposeSecret, SecretString};
use tokio::time::{Duration, sleep};
use tokio_postgres::NoTls;
use uuid::Uuid;

fn postgres_test_config() -> Option<DatabaseConfig> {
    let url = std::env::var("IRONCLAW_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;

    Some(DatabaseConfig {
        backend: DatabaseBackend::Postgres,
        url: SecretString::from(url),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        libsql_path: None,
        libsql_url: None,
        libsql_auth_token: None,
    })
}

async fn postgres_backend() -> Option<PgBackend> {
    let Some(config) = postgres_test_config() else {
        eprintln!("skipping: IRONCLAW_PG_TEST_DATABASE_URL or DATABASE_URL not configured");
        return None;
    };

    match PgBackend::new(&config).await {
        Ok(backend) => Some(backend),
        Err(e) => {
            eprintln!("skipping: database unavailable ({e})");
            None
        }
    }
}

async fn single_connection_postgres_backend() -> Option<PgBackend> {
    let Some(mut config) = postgres_test_config() else {
        eprintln!("skipping: IRONCLAW_PG_TEST_DATABASE_URL or DATABASE_URL not configured");
        return None;
    };
    config.pool_size = 1;

    match PgBackend::new(&config).await {
        Ok(backend) => Some(backend),
        Err(e) => {
            eprintln!("skipping: database unavailable ({e})");
            None
        }
    }
}

fn sample_submission(tenant_id: &str, submission_id: Uuid) -> TraceSubmissionWrite {
    let mut redaction_counts = BTreeMap::new();
    redaction_counts.insert("secret".to_string(), 1);

    TraceSubmissionWrite {
        tenant_id: tenant_id.to_string(),
        submission_id,
        trace_id: Uuid::new_v4(),
        auth_principal_ref: format!("principal:{tenant_id}"),
        contributor_pseudonym: Some(format!("contributor:{tenant_id}")),
        submitted_tenant_scope_ref: Some(tenant_id.to_string()),
        schema_version: "ironclaw.trace_contribution.v1".to_string(),
        consent_policy_version: "2026-04-24".to_string(),
        consent_scopes: vec!["debugging_evaluation".to_string()],
        allowed_uses: vec!["debugging".to_string()],
        retention_policy_id: "private_corpus_revocable".to_string(),
        status: TraceCorpusStatus::Accepted,
        privacy_risk: "low".to_string(),
        redaction_pipeline_version: "deterministic-v1".to_string(),
        redaction_counts,
        redaction_hash: format!("sha256:redaction:{tenant_id}"),
        canonical_summary_hash: Some(format!("sha256:summary:{tenant_id}")),
        submission_score: Some(0.5),
        credit_points_pending: Some(1.0),
        credit_points_final: None,
        expires_at: None,
    }
}

fn ready_rls_diagnostics() -> TraceCorpusRlsDiagnostics {
    TraceCorpusRlsDiagnostics {
        expected_table_count: 2,
        rls_enabled_count: 2,
        force_rls_enabled_count: 0,
        policy_installed_count: 2,
        missing_policy_tables: Vec::new(),
        rls_disabled_tables: Vec::new(),
        policy_expression_mismatch_tables: Vec::new(),
        current_role_bypasses_rls: false,
    }
}

fn sample_audit_event(
    tenant_id: &str,
    submission_id: Uuid,
    previous_event_hash: &str,
    event_hash: &str,
) -> TraceAuditEventWrite {
    TraceAuditEventWrite {
        tenant_id: tenant_id.to_string(),
        audit_event_id: Uuid::new_v4(),
        submission_id: Some(submission_id),
        actor_principal_ref: format!("principal:{tenant_id}"),
        actor_role: "contributor".to_string(),
        action: TraceAuditAction::Submit,
        reason: None,
        request_id: Some(format!("request:{event_hash}")),
        object_ref_id: None,
        export_manifest_id: None,
        decision_inputs_hash: None,
        previous_event_hash: Some(previous_event_hash.to_string()),
        event_hash: Some(event_hash.to_string()),
        canonical_event_json: Some(format!("{{\"event_hash\":\"{event_hash}\"}}")),
        metadata: TraceAuditSafeMetadata::Submission {
            status: TraceCorpusStatus::Accepted,
            privacy_risk: "low".to_string(),
        },
    }
}

fn sample_unhashed_audit_event(tenant_id: &str, submission_id: Uuid) -> TraceAuditEventWrite {
    TraceAuditEventWrite {
        tenant_id: tenant_id.to_string(),
        audit_event_id: Uuid::new_v4(),
        submission_id: Some(submission_id),
        actor_principal_ref: format!("principal:{tenant_id}"),
        actor_role: "system".to_string(),
        action: TraceAuditAction::Review,
        reason: Some("db_native_review_projection".to_string()),
        request_id: None,
        object_ref_id: None,
        export_manifest_id: None,
        decision_inputs_hash: None,
        previous_event_hash: None,
        event_hash: None,
        canonical_event_json: None,
        metadata: TraceAuditSafeMetadata::ReviewDecision {
            decision: "accepted".to_string(),
            resulting_status: TraceCorpusStatus::Accepted,
            reason_code: Some("db_native_review_projection".to_string()),
        },
    }
}

fn sample_credit_event(
    tenant_id: &str,
    submission_id: Uuid,
    trace_id: Uuid,
    credit_event_id: Uuid,
) -> TraceCreditEventWrite {
    TraceCreditEventWrite {
        tenant_id: tenant_id.to_string(),
        credit_event_id,
        submission_id,
        trace_id,
        credit_account_ref: format!("credit-account:{tenant_id}"),
        event_type: TraceCreditEventType::Accepted,
        points_delta: "1.0".to_string(),
        reason: format!("accepted submission for {tenant_id}"),
        external_ref: Some(format!("external:{tenant_id}:{credit_event_id}")),
        actor_principal_ref: format!("principal:{tenant_id}"),
        actor_role: "system".to_string(),
        settlement_state: TraceCreditSettlementState::Pending,
    }
}

#[derive(Clone, Copy)]
struct RawTraceRlsIds {
    submission_id: Uuid,
    object_ref_id: Uuid,
    export_manifest_id: Uuid,
    credit_event_id: Uuid,
    tombstone_id: Uuid,
    retention_job_id: Uuid,
}

#[derive(Debug, PartialEq, Eq)]
struct RawTraceRlsCounts {
    submissions: i64,
    object_refs: i64,
    export_manifests: i64,
    export_manifest_items: i64,
    credit_events: i64,
    tombstones: i64,
    retention_jobs: i64,
    retention_job_items: i64,
}

impl RawTraceRlsCounts {
    fn all(count: i64) -> Self {
        Self {
            submissions: count,
            object_refs: count,
            export_manifests: count,
            export_manifest_items: count,
            credit_events: count,
            tombstones: count,
            retention_jobs: count,
            retention_job_items: count,
        }
    }
}

async fn current_role_bypasses_trace_rls(
    client: &mut tokio_postgres::Client,
) -> Result<bool, tokio_postgres::Error> {
    let row = client
        .query_one(
            "SELECT
                EXISTS (
                    SELECT 1
                    FROM pg_class c
                    JOIN pg_roles r ON r.oid = c.relowner
                    WHERE c.relname = 'trace_submissions'
                      AND r.rolname = current_user
                ) AS is_table_owner,
                COALESCE((
                    SELECT rolsuper OR rolbypassrls
                    FROM pg_roles
                    WHERE rolname = current_user
                ), false) AS bypass_role",
            &[],
        )
        .await?;
    Ok(row.get::<_, bool>("is_table_owner") || row.get::<_, bool>("bypass_role"))
}

async fn assert_raw_sql_rls_filters_by_tenant_context(
    database_url: &str,
    tenant_a: &str,
    tenant_b: &str,
    submission_id: Uuid,
) {
    let (mut client, connection) = match tokio_postgres::connect(database_url, NoTls).await {
        Ok(parts) => parts,
        Err(e) => {
            eprintln!("skipping raw RLS assertion: database unavailable ({e})");
            return;
        }
    };
    tokio::spawn(async move {
        let _ = connection.await;
    });

    match current_role_bypasses_trace_rls(&mut client).await {
        Ok(true) => {
            eprintln!("skipping raw RLS assertion: current role bypasses RLS");
            return;
        }
        Ok(false) => {}
        Err(e) => {
            eprintln!("skipping raw RLS assertion: could not inspect role ({e})");
            return;
        }
    }

    let tx = client
        .transaction()
        .await
        .expect("start raw RLS assertion transaction");
    tx.execute(
        "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
        &[&tenant_a],
    )
    .await
    .expect("set tenant context");
    let tenant_a_count: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM trace_submissions WHERE submission_id = $1",
            &[&submission_id],
        )
        .await
        .expect("count tenant A visible submissions")
        .get(0);
    tx.execute(
        "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
        &[&tenant_b],
    )
    .await
    .expect("switch tenant context");
    let tenant_b_count: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM trace_submissions WHERE submission_id = $1",
            &[&submission_id],
        )
        .await
        .expect("count tenant B visible submissions")
        .get(0);
    tx.commit().await.expect("commit raw RLS assertion");

    assert_eq!(tenant_a_count, 1);
    assert_eq!(tenant_b_count, 1);
}

async fn raw_trace_rls_counts(
    tx: &tokio_postgres::Transaction<'_>,
    ids: RawTraceRlsIds,
) -> RawTraceRlsCounts {
    let row = tx
        .query_one(
            "SELECT
                (SELECT COUNT(*) FROM trace_submissions WHERE submission_id = $1) AS submissions,
                (SELECT COUNT(*) FROM trace_object_refs WHERE object_ref_id = $2) AS object_refs,
                (SELECT COUNT(*) FROM trace_export_manifests WHERE export_manifest_id = $3) AS export_manifests,
                (SELECT COUNT(*) FROM trace_export_manifest_items WHERE export_manifest_id = $3) AS export_manifest_items,
                (SELECT COUNT(*) FROM trace_credit_ledger WHERE credit_event_id = $4) AS credit_events,
                (SELECT COUNT(*) FROM trace_tombstones WHERE tombstone_id = $5) AS tombstones,
                (SELECT COUNT(*) FROM trace_retention_jobs WHERE retention_job_id = $6) AS retention_jobs,
                (SELECT COUNT(*) FROM trace_retention_job_items WHERE retention_job_id = $6) AS retention_job_items",
            &[
                &ids.submission_id,
                &ids.object_ref_id,
                &ids.export_manifest_id,
                &ids.credit_event_id,
                &ids.tombstone_id,
                &ids.retention_job_id,
            ],
        )
        .await
        .expect("count raw Trace Commons rows under RLS");

    RawTraceRlsCounts {
        submissions: row.get("submissions"),
        object_refs: row.get("object_refs"),
        export_manifests: row.get("export_manifests"),
        export_manifest_items: row.get("export_manifest_items"),
        credit_events: row.get("credit_events"),
        tombstones: row.get("tombstones"),
        retention_jobs: row.get("retention_jobs"),
        retention_job_items: row.get("retention_job_items"),
    }
}

async fn assert_raw_sql_trace_rows_visible_only_with_matching_tenant_context(
    database_url: &str,
    tenant_a: &str,
    tenant_b: &str,
    tenant_a_ids: RawTraceRlsIds,
    tenant_b_ids: RawTraceRlsIds,
) {
    let (mut client, connection) = match tokio_postgres::connect(database_url, NoTls).await {
        Ok(parts) => parts,
        Err(e) => {
            eprintln!("skipping raw RLS assertion: database unavailable ({e})");
            return;
        }
    };
    tokio::spawn(async move {
        let _ = connection.await;
    });

    match current_role_bypasses_trace_rls(&mut client).await {
        Ok(true) => {
            eprintln!("skipping raw RLS assertion: current role bypasses RLS");
            return;
        }
        Ok(false) => {}
        Err(e) => {
            eprintln!("skipping raw RLS assertion: could not inspect role ({e})");
            return;
        }
    }

    let tx = client
        .transaction()
        .await
        .expect("start raw no-context RLS assertion transaction");
    assert_eq!(
        raw_trace_rls_counts(&tx, tenant_a_ids).await,
        RawTraceRlsCounts::all(0),
        "tenant A rows must be invisible without transaction-local tenant context"
    );
    assert_eq!(
        raw_trace_rls_counts(&tx, tenant_b_ids).await,
        RawTraceRlsCounts::all(0),
        "tenant B rows must be invisible without transaction-local tenant context"
    );
    tx.commit()
        .await
        .expect("commit raw no-context RLS assertion");

    let tx = client
        .transaction()
        .await
        .expect("start raw tenant A RLS assertion transaction");
    tx.execute(
        "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
        &[&tenant_a],
    )
    .await
    .expect("set tenant A context");
    assert_eq!(
        raw_trace_rls_counts(&tx, tenant_a_ids).await,
        RawTraceRlsCounts::all(1),
        "tenant A rows must be visible with matching tenant context"
    );
    assert_eq!(
        raw_trace_rls_counts(&tx, tenant_b_ids).await,
        RawTraceRlsCounts::all(0),
        "tenant B rows must be invisible from tenant A context"
    );
    tx.commit()
        .await
        .expect("commit raw tenant A RLS assertion");

    let tx = client
        .transaction()
        .await
        .expect("start raw tenant B RLS assertion transaction");
    tx.execute(
        "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
        &[&tenant_b],
    )
    .await
    .expect("set tenant B context");
    assert_eq!(
        raw_trace_rls_counts(&tx, tenant_b_ids).await,
        RawTraceRlsCounts::all(1),
        "tenant B rows must be visible with matching tenant context"
    );
    assert_eq!(
        raw_trace_rls_counts(&tx, tenant_a_ids).await,
        RawTraceRlsCounts::all(0),
        "tenant A rows must be invisible from tenant B context"
    );
    tx.commit()
        .await
        .expect("commit raw tenant B RLS assertion");
}

async fn assert_trace_rls_policies_installed(backend: &PgBackend) {
    let expected_tables = vec![
        "trace_tenants".to_string(),
        "trace_tenant_policies".to_string(),
        "trace_submissions".to_string(),
        "trace_object_refs".to_string(),
        "trace_derived_records".to_string(),
        "trace_audit_events".to_string(),
        "trace_credit_ledger".to_string(),
        "trace_tombstones".to_string(),
        "trace_vector_entries".to_string(),
        "trace_export_manifests".to_string(),
        "trace_export_manifest_items".to_string(),
        "trace_retention_jobs".to_string(),
        "trace_retention_job_items".to_string(),
    ];
    let client = backend.pool().get().await.expect("get policy connection");
    let rows = client
        .query(
            "SELECT tablename
             FROM pg_policies
             WHERE schemaname = current_schema()
               AND policyname = 'trace_corpus_tenant_isolation'
               AND tablename = ANY($1)",
            &[&expected_tables],
        )
        .await
        .expect("read trace RLS policies");
    let mut actual_tables: Vec<String> = rows.iter().map(|row| row.get("tablename")).collect();
    actual_tables.sort();

    let mut expected_tables = expected_tables;
    expected_tables.sort();
    assert_eq!(actual_tables, expected_tables);
}

#[test]
fn trace_corpus_rls_diagnostics_ready_requires_complete_safe_policy_state() {
    assert!(ready_rls_diagnostics().rls_ready());

    let mut missing_policy = ready_rls_diagnostics();
    missing_policy.policy_installed_count = 1;
    missing_policy
        .missing_policy_tables
        .push("trace_submissions".to_string());
    assert!(!missing_policy.rls_ready());

    let mut disabled_rls = ready_rls_diagnostics();
    disabled_rls.rls_enabled_count = 1;
    disabled_rls
        .rls_disabled_tables
        .push("trace_object_refs".to_string());
    assert!(!disabled_rls.rls_ready());

    let mut expression_mismatch = ready_rls_diagnostics();
    expression_mismatch
        .policy_expression_mismatch_tables
        .push("trace_credit_ledger".to_string());
    assert!(!expression_mismatch.rls_ready());

    let mut bypass_role = ready_rls_diagnostics();
    bypass_role.current_role_bypasses_rls = true;
    assert!(!bypass_role.rls_ready());
}

#[tokio::test]
async fn pg_store_rejects_stale_audit_previous_hash_per_tenant() {
    let Some(backend) = single_connection_postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");
    assert_trace_rls_policies_installed(&backend).await;

    let tenant_id = format!("pg-audit-chain-{}", Uuid::new_v4());
    let other_tenant_id = format!("pg-audit-chain-other-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let other_submission_id = Uuid::new_v4();

    backend
        .upsert_trace_submission(sample_submission(&tenant_id, submission_id))
        .await
        .expect("insert submission");
    backend
        .upsert_trace_submission(sample_submission(&other_tenant_id, other_submission_id))
        .await
        .expect("insert other tenant submission");

    backend
        .append_trace_audit_event(sample_unhashed_audit_event(&tenant_id, submission_id))
        .await
        .expect("append DB-native unhashed audit event");
    backend
        .append_trace_audit_event(sample_audit_event(
            &tenant_id,
            submission_id,
            "sha256:file-only-predecessor",
            "sha256:first",
        ))
        .await
        .expect("append first mirrored hash-chain segment");
    backend
        .append_trace_audit_event(sample_audit_event(
            &tenant_id,
            submission_id,
            "sha256:first",
            "sha256:second",
        ))
        .await
        .expect("append second audit event");

    let stale_append = backend
        .append_trace_audit_event(sample_audit_event(
            &tenant_id,
            submission_id,
            "sha256:file-only-predecessor",
            "sha256:stale",
        ))
        .await;
    assert!(
        stale_append.is_err(),
        "stale audit hash-chain predecessor must be rejected"
    );

    let audit_events = backend
        .list_trace_audit_events(&tenant_id)
        .await
        .expect("list audit events");
    assert_eq!(audit_events.len(), 3);
    assert_eq!(
        audit_events
            .iter()
            .map(|event| event.event_hash.as_deref())
            .collect::<Vec<_>>(),
        vec![None, Some("sha256:first"), Some("sha256:second")]
    );
    assert_eq!(
        audit_events
            .iter()
            .map(|event| event.audit_sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );

    backend
        .append_trace_audit_event(sample_audit_event(
            &other_tenant_id,
            other_submission_id,
            "sha256:genesis",
            "sha256:first-other-tenant",
        ))
        .await
        .expect("other tenant starts an independent audit chain");

    let mut client = backend.pool().get().await.expect("get cleanup connection");
    for tenant_id in [&tenant_id, &other_tenant_id] {
        let tx = client
            .transaction()
            .await
            .expect("start cleanup transaction");
        tx.execute(
            "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
            &[tenant_id],
        )
        .await
        .expect("set cleanup tenant context");
        let _ = tx
            .execute(
                "DELETE FROM trace_tenants WHERE tenant_id = $1",
                &[tenant_id],
            )
            .await;
        tx.commit().await.expect("commit cleanup transaction");
    }
}

#[tokio::test]
async fn store_facade_sets_transaction_local_tenant_context() {
    let Some(backend) = single_connection_postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");
    assert_trace_rls_policies_installed(&backend).await;

    let tenant_a = format!("rls-context-a-{}", Uuid::new_v4());
    let tenant_b = format!("rls-context-b-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();

    {
        let client = backend.pool().get().await.expect("get pooled connection");
        client
            .execute(
                "SELECT set_config('ironclaw.trace_tenant_id', $1, false)",
                &[&tenant_b],
            )
            .await
            .expect("poison pooled tenant context");
    }

    let inserted_a = backend
        .upsert_trace_submission(sample_submission(&tenant_a, submission_id))
        .await
        .expect("insert tenant A submission despite stale session context");
    assert_eq!(inserted_a.tenant_id, tenant_a);

    let fetched_a = backend
        .get_trace_submission(&tenant_a, submission_id)
        .await
        .expect("get tenant A submission despite stale session context")
        .expect("tenant A submission exists");
    assert_eq!(fetched_a.tenant_id, tenant_a);

    let mut client = backend.pool().get().await.expect("get pooled connection");
    let tenant_context: String = client
        .query_one(
            "SELECT current_setting('ironclaw.trace_tenant_id', true)",
            &[],
        )
        .await
        .expect("read pooled tenant context")
        .get(0);
    assert_eq!(tenant_context, tenant_b);

    let role_bypasses_rls = current_role_bypasses_trace_rls(&mut client)
        .await
        .unwrap_or_else(|e| {
            eprintln!("skipping RLS role assertion: could not inspect role ({e})");
            true
        });
    if role_bypasses_rls {
        eprintln!(
            "RLS role bypasses table policies; this test verifies transaction-local context cleanup, not policy enforcement"
        );
    }

    let tx = client
        .transaction()
        .await
        .expect("start cleanup transaction");
    tx.execute(
        "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
        &[&tenant_a],
    )
    .await
    .expect("set cleanup tenant context");
    let _ = tx
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = $1",
            &[&tenant_a],
        )
        .await;
    tx.commit().await.expect("commit cleanup transaction");
}

#[tokio::test]
async fn store_facade_keeps_same_submission_id_isolated_by_tenant() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");
    assert_trace_rls_policies_installed(&backend).await;

    let tenant_a = format!("rls-tenant-a-{}", Uuid::new_v4());
    let tenant_b = format!("rls-tenant-b-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();

    let inserted_a = backend
        .upsert_trace_submission(sample_submission(&tenant_a, submission_id))
        .await
        .expect("insert tenant A submission");
    let inserted_b = backend
        .upsert_trace_submission(sample_submission(&tenant_b, submission_id))
        .await
        .expect("insert tenant B submission with same submission id");

    assert_eq!(inserted_a.submission_id, submission_id);
    assert_eq!(inserted_b.submission_id, submission_id);
    assert_eq!(inserted_a.tenant_id, tenant_a);
    assert_eq!(inserted_b.tenant_id, tenant_b);
    assert_ne!(inserted_a.trace_id, inserted_b.trace_id);

    let tenant_a_submission = backend
        .get_trace_submission(&tenant_a, submission_id)
        .await
        .expect("get tenant A submission")
        .expect("tenant A submission exists");
    let tenant_b_submission = backend
        .get_trace_submission(&tenant_b, submission_id)
        .await
        .expect("get tenant B submission")
        .expect("tenant B submission exists");

    assert_eq!(tenant_a_submission.tenant_id, tenant_a);
    assert_eq!(tenant_b_submission.tenant_id, tenant_b);
    assert_eq!(
        tenant_a_submission.contributor_pseudonym.as_deref(),
        Some(format!("contributor:{tenant_a}").as_str())
    );
    assert_eq!(
        tenant_b_submission.contributor_pseudonym.as_deref(),
        Some(format!("contributor:{tenant_b}").as_str())
    );

    let listed_a = backend
        .list_trace_submissions(&tenant_a)
        .await
        .expect("list tenant A submissions");
    let listed_b = backend
        .list_trace_submissions(&tenant_b)
        .await
        .expect("list tenant B submissions");

    assert_eq!(listed_a.len(), 1);
    assert_eq!(listed_b.len(), 1);
    assert_eq!(listed_a[0].tenant_id, tenant_a);
    assert_eq!(listed_b[0].tenant_id, tenant_b);
    assert_ne!(listed_a[0].trace_id, listed_b[0].trace_id);

    if let Some(config) = postgres_test_config() {
        assert_raw_sql_rls_filters_by_tenant_context(
            config.url.expose_secret(),
            &tenant_a,
            &tenant_b,
            submission_id,
        )
        .await;
    }

    let mut client = backend.pool().get().await.expect("get cleanup connection");
    for tenant_id in [tenant_a, tenant_b] {
        let tx = client
            .transaction()
            .await
            .expect("start cleanup transaction");
        tx.execute(
            "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await
        .expect("set cleanup tenant context");
        let _ = tx
            .execute(
                "DELETE FROM trace_tenants WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await;
        tx.commit().await.expect("commit cleanup transaction");
    }
}

#[tokio::test]
async fn store_facade_preserves_retention_job_scope_and_items() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");
    assert_trace_rls_policies_installed(&backend).await;

    let tenant_alpha = format!("rls-retention-alpha-{}", Uuid::new_v4());
    let tenant_beta = format!("rls-retention-beta-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();

    backend
        .upsert_trace_submission(sample_submission(&tenant_alpha, submission_id))
        .await
        .expect("insert alpha submission");
    backend
        .upsert_trace_submission(sample_submission(&tenant_beta, submission_id))
        .await
        .expect("insert beta submission with same submission id");

    let retention_job_id = Uuid::new_v4();
    let mut action_counts = BTreeMap::new();
    action_counts.insert("records_marked_expired".to_string(), 1);
    action_counts.insert("records_marked_purged".to_string(), 1);
    let job = backend
        .upsert_trace_retention_job(TraceRetentionJobWrite {
            tenant_id: tenant_alpha.clone(),
            retention_job_id,
            purpose: "test_pg_retention_purge".to_string(),
            dry_run: false,
            status: TraceRetentionJobStatus::Running,
            requested_by_principal_ref: "principal:retention-worker".to_string(),
            requested_by_role: "retention_worker".to_string(),
            purge_expired_before: Some(Utc::now()),
            prune_export_cache: true,
            max_export_age_hours: Some(24),
            audit_event_id: Some(Uuid::new_v4()),
            action_counts: action_counts.clone(),
            selected_revoked_count: 0,
            selected_expired_count: 1,
            started_at: Some(Utc::now()),
            completed_at: None,
        })
        .await
        .expect("insert alpha retention job");
    assert_eq!(job.tenant_id, tenant_alpha);
    assert_eq!(job.retention_job_id, retention_job_id);
    assert_eq!(job.status, TraceRetentionJobStatus::Running);
    assert_eq!(job.action_counts, action_counts);

    action_counts.insert("records_marked_purged".to_string(), 2);
    let updated_job = backend
        .upsert_trace_retention_job(TraceRetentionJobWrite {
            tenant_id: tenant_alpha.clone(),
            retention_job_id,
            purpose: "test_pg_retention_purge".to_string(),
            dry_run: false,
            status: TraceRetentionJobStatus::Complete,
            requested_by_principal_ref: "principal:retention-worker".to_string(),
            requested_by_role: "retention_worker".to_string(),
            purge_expired_before: Some(Utc::now()),
            prune_export_cache: true,
            max_export_age_hours: Some(24),
            audit_event_id: job.audit_event_id,
            action_counts: action_counts.clone(),
            selected_revoked_count: 0,
            selected_expired_count: 2,
            started_at: job.started_at,
            completed_at: Some(Utc::now()),
        })
        .await
        .expect("idempotently update alpha retention job");
    assert_eq!(updated_job.retention_job_id, retention_job_id);
    assert_eq!(updated_job.status, TraceRetentionJobStatus::Complete);
    assert_eq!(updated_job.action_counts, action_counts);
    assert_eq!(updated_job.selected_expired_count, 2);

    let mut item_counts = BTreeMap::new();
    item_counts.insert("object_refs_invalidated".to_string(), 1);
    item_counts.insert("derived_records_invalidated".to_string(), 1);
    let item = backend
        .upsert_trace_retention_job_item(TraceRetentionJobItemWrite {
            tenant_id: tenant_alpha.clone(),
            retention_job_id,
            submission_id,
            action: TraceRetentionJobItemAction::Purge,
            status: TraceRetentionJobItemStatus::Pending,
            reason: "retention_pending".to_string(),
            action_counts: item_counts.clone(),
            verified_at: None,
        })
        .await
        .expect("insert alpha retention job item");
    assert_eq!(item.tenant_id, tenant_alpha);
    assert_eq!(item.submission_id, submission_id);
    assert_eq!(item.action, TraceRetentionJobItemAction::Purge);
    assert_eq!(item.status, TraceRetentionJobItemStatus::Pending);

    item_counts.insert("records_marked_purged".to_string(), 1);
    let updated_item = backend
        .upsert_trace_retention_job_item(TraceRetentionJobItemWrite {
            tenant_id: tenant_alpha.clone(),
            retention_job_id,
            submission_id,
            action: TraceRetentionJobItemAction::Purge,
            status: TraceRetentionJobItemStatus::Done,
            reason: "retention_purged".to_string(),
            action_counts: item_counts.clone(),
            verified_at: Some(Utc::now()),
        })
        .await
        .expect("idempotently update alpha retention job item");
    assert_eq!(updated_item.status, TraceRetentionJobItemStatus::Done);
    assert_eq!(updated_item.reason, "retention_purged");
    assert_eq!(updated_item.action_counts, item_counts);

    let alpha_jobs = backend
        .list_trace_retention_jobs(&tenant_alpha)
        .await
        .expect("list alpha retention jobs");
    assert_eq!(alpha_jobs.len(), 1);
    assert_eq!(alpha_jobs[0].retention_job_id, retention_job_id);
    assert_eq!(alpha_jobs[0].status, TraceRetentionJobStatus::Complete);
    let beta_jobs = backend
        .list_trace_retention_jobs(&tenant_beta)
        .await
        .expect("list beta retention jobs");
    assert!(beta_jobs.is_empty());

    let alpha_items = backend
        .list_trace_retention_job_items(&tenant_alpha, retention_job_id)
        .await
        .expect("list alpha retention job items");
    assert_eq!(alpha_items.len(), 1);
    assert_eq!(alpha_items[0].submission_id, submission_id);
    assert_eq!(alpha_items[0].status, TraceRetentionJobItemStatus::Done);
    let beta_items = backend
        .list_trace_retention_job_items(&tenant_beta, retention_job_id)
        .await
        .expect("list beta retention job items");
    assert!(beta_items.is_empty());

    let mut client = backend.pool().get().await.expect("get cleanup connection");
    for tenant_id in [&tenant_alpha, &tenant_beta] {
        let tx = client
            .transaction()
            .await
            .expect("start cleanup transaction");
        tx.execute(
            "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
            &[tenant_id],
        )
        .await
        .expect("set cleanup tenant context");
        let _ = tx
            .execute(
                "DELETE FROM trace_tenants WHERE tenant_id = $1",
                &[tenant_id],
            )
            .await;
        tx.commit().await.expect("commit cleanup transaction");
    }
}

#[tokio::test]
async fn raw_trace_corpus_rls_requires_matching_transaction_local_tenant_context() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");
    assert_trace_rls_policies_installed(&backend).await;

    let tenant_a = format!("rls-raw-a-{}", Uuid::new_v4());
    let tenant_b = format!("rls-raw-b-{}", Uuid::new_v4());
    let tenant_a_submission_id = Uuid::new_v4();
    let tenant_b_submission_id = Uuid::new_v4();
    let tenant_a_trace_id = Uuid::new_v4();
    let tenant_b_trace_id = Uuid::new_v4();

    let mut tenant_a_submission = sample_submission(&tenant_a, tenant_a_submission_id);
    tenant_a_submission.trace_id = tenant_a_trace_id;
    backend
        .upsert_trace_submission(tenant_a_submission)
        .await
        .expect("insert tenant A submission");
    let mut tenant_b_submission = sample_submission(&tenant_b, tenant_b_submission_id);
    tenant_b_submission.trace_id = tenant_b_trace_id;
    backend
        .upsert_trace_submission(tenant_b_submission)
        .await
        .expect("insert tenant B submission");

    let tenant_a_object_ref_id = Uuid::new_v4();
    backend
        .append_trace_object_ref(TraceObjectRefWrite {
            tenant_id: tenant_a.clone(),
            object_ref_id: tenant_a_object_ref_id,
            submission_id: tenant_a_submission_id,
            artifact_kind: TraceObjectArtifactKind::SubmittedEnvelope,
            object_store: "s3://private-corpus".to_string(),
            object_key: format!("{tenant_a}/submission.json"),
            content_sha256: format!("sha256:{tenant_a}:object"),
            encryption_key_ref: format!("kms:{tenant_a}"),
            size_bytes: 4096,
            compression: None,
            created_by_job_id: None,
        })
        .await
        .expect("append tenant A object ref");
    let tenant_b_object_ref_id = Uuid::new_v4();
    backend
        .append_trace_object_ref(TraceObjectRefWrite {
            tenant_id: tenant_b.clone(),
            object_ref_id: tenant_b_object_ref_id,
            submission_id: tenant_b_submission_id,
            artifact_kind: TraceObjectArtifactKind::SubmittedEnvelope,
            object_store: "s3://private-corpus".to_string(),
            object_key: format!("{tenant_b}/submission.json"),
            content_sha256: format!("sha256:{tenant_b}:object"),
            encryption_key_ref: format!("kms:{tenant_b}"),
            size_bytes: 2048,
            compression: None,
            created_by_job_id: None,
        })
        .await
        .expect("append tenant B object ref");

    let tenant_a_export_manifest_id = Uuid::new_v4();
    backend
        .upsert_trace_export_manifest(TraceExportManifestWrite {
            tenant_id: tenant_a.clone(),
            export_manifest_id: tenant_a_export_manifest_id,
            artifact_kind: TraceObjectArtifactKind::ExportArtifact,
            purpose_code: Some("rls_replay_dataset".to_string()),
            audit_event_id: Some(Uuid::new_v4()),
            source_submission_ids: vec![tenant_a_submission_id],
            source_submission_ids_hash: format!("sha256:{tenant_a}:sources"),
            item_count: 1,
            generated_at: Utc::now(),
        })
        .await
        .expect("append tenant A export manifest");
    backend
        .upsert_trace_export_manifest_item(TraceExportManifestItemWrite {
            tenant_id: tenant_a.clone(),
            export_manifest_id: tenant_a_export_manifest_id,
            submission_id: tenant_a_submission_id,
            trace_id: tenant_a_trace_id,
            derived_id: None,
            object_ref_id: Some(tenant_a_object_ref_id),
            vector_entry_id: None,
            source_status_at_export: TraceCorpusStatus::Accepted,
            source_hash_at_export: format!("sha256:{tenant_a}:source"),
        })
        .await
        .expect("append tenant A export manifest item");
    let tenant_b_export_manifest_id = Uuid::new_v4();
    backend
        .upsert_trace_export_manifest(TraceExportManifestWrite {
            tenant_id: tenant_b.clone(),
            export_manifest_id: tenant_b_export_manifest_id,
            artifact_kind: TraceObjectArtifactKind::ExportArtifact,
            purpose_code: Some("rls_replay_dataset".to_string()),
            audit_event_id: Some(Uuid::new_v4()),
            source_submission_ids: vec![tenant_b_submission_id],
            source_submission_ids_hash: format!("sha256:{tenant_b}:sources"),
            item_count: 1,
            generated_at: Utc::now(),
        })
        .await
        .expect("append tenant B export manifest");
    backend
        .upsert_trace_export_manifest_item(TraceExportManifestItemWrite {
            tenant_id: tenant_b.clone(),
            export_manifest_id: tenant_b_export_manifest_id,
            submission_id: tenant_b_submission_id,
            trace_id: tenant_b_trace_id,
            derived_id: None,
            object_ref_id: Some(tenant_b_object_ref_id),
            vector_entry_id: None,
            source_status_at_export: TraceCorpusStatus::Accepted,
            source_hash_at_export: format!("sha256:{tenant_b}:source"),
        })
        .await
        .expect("append tenant B export manifest item");

    let tenant_a_credit_event_id = Uuid::new_v4();
    backend
        .append_trace_credit_event(sample_credit_event(
            &tenant_a,
            tenant_a_submission_id,
            tenant_a_trace_id,
            tenant_a_credit_event_id,
        ))
        .await
        .expect("append tenant A credit event");
    let tenant_b_credit_event_id = Uuid::new_v4();
    backend
        .append_trace_credit_event(sample_credit_event(
            &tenant_b,
            tenant_b_submission_id,
            tenant_b_trace_id,
            tenant_b_credit_event_id,
        ))
        .await
        .expect("append tenant B credit event");

    let effective_at = DateTime::parse_from_rfc3339("2026-04-25T12:00:00Z")
        .expect("parse effective timestamp")
        .with_timezone(&Utc);
    let tenant_a_tombstone_id = Uuid::new_v4();
    backend
        .write_trace_tombstone(TraceTombstoneWrite {
            tombstone_id: tenant_a_tombstone_id,
            tenant_id: tenant_a.clone(),
            submission_id: tenant_a_submission_id,
            trace_id: Some(tenant_a_trace_id),
            redaction_hash: Some(format!("sha256:{tenant_a}:redaction")),
            canonical_summary_hash: Some(format!("sha256:{tenant_a}:summary")),
            reason: "tenant A revocation".to_string(),
            effective_at,
            retain_until: None,
            created_by_principal_ref: format!("principal:{tenant_a}"),
        })
        .await
        .expect("write tenant A tombstone");
    let tenant_b_tombstone_id = Uuid::new_v4();
    backend
        .write_trace_tombstone(TraceTombstoneWrite {
            tombstone_id: tenant_b_tombstone_id,
            tenant_id: tenant_b.clone(),
            submission_id: tenant_b_submission_id,
            trace_id: Some(tenant_b_trace_id),
            redaction_hash: Some(format!("sha256:{tenant_b}:redaction")),
            canonical_summary_hash: Some(format!("sha256:{tenant_b}:summary")),
            reason: "tenant B revocation".to_string(),
            effective_at,
            retain_until: None,
            created_by_principal_ref: format!("principal:{tenant_b}"),
        })
        .await
        .expect("write tenant B tombstone");

    let mut tenant_a_retention_action_counts = BTreeMap::new();
    tenant_a_retention_action_counts.insert("records_marked_expired".to_string(), 1);
    let tenant_a_retention_job_id = Uuid::new_v4();
    backend
        .upsert_trace_retention_job(TraceRetentionJobWrite {
            tenant_id: tenant_a.clone(),
            retention_job_id: tenant_a_retention_job_id,
            purpose: "rls_retention_a".to_string(),
            dry_run: false,
            status: TraceRetentionJobStatus::Complete,
            requested_by_principal_ref: format!("principal:{tenant_a}"),
            requested_by_role: "retention_worker".to_string(),
            purge_expired_before: Some(effective_at),
            prune_export_cache: true,
            max_export_age_hours: Some(24),
            audit_event_id: Some(Uuid::new_v4()),
            action_counts: tenant_a_retention_action_counts,
            selected_revoked_count: 0,
            selected_expired_count: 1,
            started_at: Some(effective_at),
            completed_at: Some(effective_at),
        })
        .await
        .expect("write tenant A retention job");
    let mut tenant_a_retention_item_counts = BTreeMap::new();
    tenant_a_retention_item_counts.insert("records_marked_expired".to_string(), 1);
    backend
        .upsert_trace_retention_job_item(TraceRetentionJobItemWrite {
            tenant_id: tenant_a.clone(),
            retention_job_id: tenant_a_retention_job_id,
            submission_id: tenant_a_submission_id,
            action: TraceRetentionJobItemAction::Expire,
            status: TraceRetentionJobItemStatus::Done,
            reason: "retention_expired".to_string(),
            action_counts: tenant_a_retention_item_counts,
            verified_at: Some(effective_at),
        })
        .await
        .expect("write tenant A retention job item");

    let mut tenant_b_retention_action_counts = BTreeMap::new();
    tenant_b_retention_action_counts.insert("records_marked_purged".to_string(), 1);
    let tenant_b_retention_job_id = Uuid::new_v4();
    backend
        .upsert_trace_retention_job(TraceRetentionJobWrite {
            tenant_id: tenant_b.clone(),
            retention_job_id: tenant_b_retention_job_id,
            purpose: "rls_retention_b".to_string(),
            dry_run: false,
            status: TraceRetentionJobStatus::Complete,
            requested_by_principal_ref: format!("principal:{tenant_b}"),
            requested_by_role: "retention_worker".to_string(),
            purge_expired_before: Some(effective_at),
            prune_export_cache: true,
            max_export_age_hours: Some(24),
            audit_event_id: Some(Uuid::new_v4()),
            action_counts: tenant_b_retention_action_counts,
            selected_revoked_count: 0,
            selected_expired_count: 1,
            started_at: Some(effective_at),
            completed_at: Some(effective_at),
        })
        .await
        .expect("write tenant B retention job");
    let mut tenant_b_retention_item_counts = BTreeMap::new();
    tenant_b_retention_item_counts.insert("records_marked_purged".to_string(), 1);
    backend
        .upsert_trace_retention_job_item(TraceRetentionJobItemWrite {
            tenant_id: tenant_b.clone(),
            retention_job_id: tenant_b_retention_job_id,
            submission_id: tenant_b_submission_id,
            action: TraceRetentionJobItemAction::Purge,
            status: TraceRetentionJobItemStatus::Done,
            reason: "retention_purged".to_string(),
            action_counts: tenant_b_retention_item_counts,
            verified_at: Some(effective_at),
        })
        .await
        .expect("write tenant B retention job item");

    assert!(
        backend
            .get_trace_submission(&tenant_b, tenant_a_submission_id)
            .await
            .expect("tenant B probes tenant A submission")
            .is_none()
    );
    assert!(
        backend
            .list_trace_object_refs(&tenant_b, tenant_a_submission_id)
            .await
            .expect("tenant B probes tenant A object refs")
            .is_empty()
    );

    let tenant_b_credit_events = backend
        .list_trace_credit_events(&tenant_b)
        .await
        .expect("list tenant B credit events");
    assert_eq!(tenant_b_credit_events.len(), 1);
    assert_eq!(
        tenant_b_credit_events[0].credit_event_id,
        tenant_b_credit_event_id
    );
    assert_ne!(
        tenant_b_credit_events[0].credit_event_id,
        tenant_a_credit_event_id
    );

    let tenant_b_tombstones = backend
        .list_trace_tombstones(&tenant_b)
        .await
        .expect("list tenant B tombstones");
    assert_eq!(tenant_b_tombstones.len(), 1);
    assert_eq!(tenant_b_tombstones[0].tombstone_id, tenant_b_tombstone_id);
    assert_ne!(tenant_b_tombstones[0].tombstone_id, tenant_a_tombstone_id);

    if let Some(config) = postgres_test_config() {
        assert_raw_sql_trace_rows_visible_only_with_matching_tenant_context(
            config.url.expose_secret(),
            &tenant_a,
            &tenant_b,
            RawTraceRlsIds {
                submission_id: tenant_a_submission_id,
                object_ref_id: tenant_a_object_ref_id,
                export_manifest_id: tenant_a_export_manifest_id,
                credit_event_id: tenant_a_credit_event_id,
                tombstone_id: tenant_a_tombstone_id,
                retention_job_id: tenant_a_retention_job_id,
            },
            RawTraceRlsIds {
                submission_id: tenant_b_submission_id,
                object_ref_id: tenant_b_object_ref_id,
                export_manifest_id: tenant_b_export_manifest_id,
                credit_event_id: tenant_b_credit_event_id,
                tombstone_id: tenant_b_tombstone_id,
                retention_job_id: tenant_b_retention_job_id,
            },
        )
        .await;
    }

    let mut client = backend.pool().get().await.expect("get cleanup connection");
    for tenant_id in [&tenant_a, &tenant_b] {
        let tx = client
            .transaction()
            .await
            .expect("start cleanup transaction");
        tx.execute(
            "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
            &[tenant_id],
        )
        .await
        .expect("set cleanup tenant context");
        let _ = tx
            .execute(
                "DELETE FROM trace_tenants WHERE tenant_id = $1",
                &[tenant_id],
            )
            .await;
        tx.commit().await.expect("commit cleanup transaction");
    }
}

#[tokio::test]
async fn pg_trace_corpus_rls_diagnostics_report_policy_coverage() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");
    assert_trace_rls_policies_installed(&backend).await;

    let diagnostics = backend
        .trace_corpus_rls_diagnostics()
        .await
        .expect("read RLS diagnostics")
        .expect("PostgreSQL reports RLS diagnostics");
    assert_eq!(diagnostics.expected_table_count, 13);
    assert_eq!(diagnostics.policy_installed_count, 13);
    assert_eq!(diagnostics.rls_enabled_count, 13);
    assert!(diagnostics.missing_policy_tables.is_empty());
    assert!(diagnostics.rls_disabled_tables.is_empty());
    assert!(diagnostics.policy_expression_mismatch_tables.is_empty());
    assert!(diagnostics.force_rls_enabled_count <= diagnostics.expected_table_count);
    assert_eq!(
        diagnostics.rls_ready(),
        !diagnostics.current_role_bypasses_rls,
        "RLS readiness should be blocked only when the current test role bypasses RLS"
    );
}

#[tokio::test]
async fn store_facade_invalidates_object_refs_and_tombstones_by_tenant_scope() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");
    assert_trace_rls_policies_installed(&backend).await;

    let tenant_a = format!("rls-objects-a-{}", Uuid::new_v4());
    let tenant_b = format!("rls-objects-b-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();

    let inserted_a = backend
        .upsert_trace_submission(sample_submission(&tenant_a, submission_id))
        .await
        .expect("insert tenant A submission");
    let inserted_b = backend
        .upsert_trace_submission(sample_submission(&tenant_b, submission_id))
        .await
        .expect("insert tenant B submission");

    let tenant_a_first_object_ref_id = Uuid::new_v4();
    backend
        .append_trace_object_ref(TraceObjectRefWrite {
            tenant_id: tenant_a.clone(),
            object_ref_id: tenant_a_first_object_ref_id,
            submission_id,
            artifact_kind: TraceObjectArtifactKind::SubmittedEnvelope,
            object_store: "s3://private-corpus".to_string(),
            object_key: format!("{tenant_a}/submission.json"),
            content_sha256: format!("sha256:{tenant_a}:object-1"),
            encryption_key_ref: format!("kms:{tenant_a}"),
            size_bytes: 4096,
            compression: None,
            created_by_job_id: None,
        })
        .await
        .expect("append tenant A first object ref");

    sleep(Duration::from_millis(5)).await;

    let tenant_a_latest_object_ref_id = Uuid::new_v4();
    backend
        .append_trace_object_ref(TraceObjectRefWrite {
            tenant_id: tenant_a.clone(),
            object_ref_id: tenant_a_latest_object_ref_id,
            submission_id,
            artifact_kind: TraceObjectArtifactKind::SubmittedEnvelope,
            object_store: "s3://private-corpus".to_string(),
            object_key: format!("{tenant_a}/submission-v2.json"),
            content_sha256: format!("sha256:{tenant_a}:object-2"),
            encryption_key_ref: format!("kms:{tenant_a}"),
            size_bytes: 8192,
            compression: Some("zstd".to_string()),
            created_by_job_id: Some(Uuid::new_v4()),
        })
        .await
        .expect("append tenant A latest object ref");

    let tenant_b_object_ref_id = Uuid::new_v4();
    backend
        .append_trace_object_ref(TraceObjectRefWrite {
            tenant_id: tenant_b.clone(),
            object_ref_id: tenant_b_object_ref_id,
            submission_id,
            artifact_kind: TraceObjectArtifactKind::SubmittedEnvelope,
            object_store: "s3://private-corpus".to_string(),
            object_key: format!("{tenant_b}/submission.json"),
            content_sha256: format!("sha256:{tenant_b}:object"),
            encryption_key_ref: format!("kms:{tenant_b}"),
            size_bytes: 2048,
            compression: None,
            created_by_job_id: None,
        })
        .await
        .expect("append tenant B object ref");

    let tenant_a_latest = backend
        .get_latest_active_trace_object_ref(
            &tenant_a,
            submission_id,
            TraceObjectArtifactKind::SubmittedEnvelope,
        )
        .await
        .expect("get tenant A latest active object ref")
        .expect("tenant A latest active object ref exists");
    assert_eq!(tenant_a_latest.object_ref_id, tenant_a_latest_object_ref_id);
    assert_eq!(
        tenant_a_latest.object_key,
        format!("{tenant_a}/submission-v2.json")
    );

    let tenant_b_latest = backend
        .get_latest_active_trace_object_ref(
            &tenant_b,
            submission_id,
            TraceObjectArtifactKind::SubmittedEnvelope,
        )
        .await
        .expect("get tenant B latest active object ref")
        .expect("tenant B latest active object ref exists");
    assert_eq!(tenant_b_latest.object_ref_id, tenant_b_object_ref_id);
    assert_eq!(
        tenant_b_latest.object_key,
        format!("{tenant_b}/submission.json")
    );

    let tenant_a_derived_id = Uuid::new_v4();
    backend
        .append_trace_derived_record(TraceDerivedRecordWrite {
            tenant_id: tenant_a.clone(),
            derived_id: tenant_a_derived_id,
            submission_id,
            trace_id: inserted_a.trace_id,
            status: TraceDerivedStatus::Current,
            worker_kind: TraceWorkerKind::DuplicatePrecheck,
            worker_version: "duplicate-worker-v1".to_string(),
            input_object_ref: Some(TenantScopedTraceObjectRef {
                tenant_id: tenant_a.clone(),
                submission_id,
                object_ref_id: tenant_a_first_object_ref_id,
            }),
            input_hash: format!("sha256:{tenant_a}:object-1"),
            output_object_ref: None,
            canonical_summary: Some("Tenant A canonical summary.".to_string()),
            canonical_summary_hash: Some(format!("sha256:{tenant_a}:summary")),
            summary_model: "summary-model-v1".to_string(),
            task_success: Some("success".to_string()),
            privacy_risk: Some("low".to_string()),
            event_count: Some(3),
            tool_sequence: vec!["calendar_create".to_string()],
            tool_categories: vec!["calendar".to_string()],
            coverage_tags: vec!["tool:calendar_create".to_string()],
            duplicate_score: Some(0.1),
            novelty_score: Some(0.7),
            cluster_id: Some(format!("cluster:{tenant_a}")),
        })
        .await
        .expect("append tenant A derived record");

    let tenant_b_derived_id = Uuid::new_v4();
    backend
        .append_trace_derived_record(TraceDerivedRecordWrite {
            tenant_id: tenant_b.clone(),
            derived_id: tenant_b_derived_id,
            submission_id,
            trace_id: inserted_b.trace_id,
            status: TraceDerivedStatus::Current,
            worker_kind: TraceWorkerKind::DuplicatePrecheck,
            worker_version: "duplicate-worker-v1".to_string(),
            input_object_ref: Some(TenantScopedTraceObjectRef {
                tenant_id: tenant_b.clone(),
                submission_id,
                object_ref_id: tenant_b_object_ref_id,
            }),
            input_hash: format!("sha256:{tenant_b}:object"),
            output_object_ref: None,
            canonical_summary: Some("Tenant B canonical summary.".to_string()),
            canonical_summary_hash: Some(format!("sha256:{tenant_b}:summary")),
            summary_model: "summary-model-v1".to_string(),
            task_success: Some("success".to_string()),
            privacy_risk: Some("low".to_string()),
            event_count: Some(2),
            tool_sequence: vec!["memory_search".to_string()],
            tool_categories: vec!["memory".to_string()],
            coverage_tags: vec!["tool:memory_search".to_string()],
            duplicate_score: Some(0.2),
            novelty_score: Some(0.5),
            cluster_id: Some(format!("cluster:{tenant_b}")),
        })
        .await
        .expect("append tenant B derived record");

    let invalidated = backend
        .invalidate_trace_submission_artifacts(
            &tenant_a,
            submission_id,
            TraceDerivedStatus::Revoked,
        )
        .await
        .expect("invalidate tenant A artifacts");
    assert_eq!(invalidated.object_refs_invalidated, 2);
    assert_eq!(invalidated.derived_records_invalidated, 1);

    let idempotent = backend
        .invalidate_trace_submission_artifacts(
            &tenant_a,
            submission_id,
            TraceDerivedStatus::Revoked,
        )
        .await
        .expect("repeat tenant A artifact invalidation");
    assert_eq!(idempotent.object_refs_invalidated, 0);
    assert_eq!(idempotent.derived_records_invalidated, 0);

    assert!(
        backend
            .get_latest_active_trace_object_ref(
                &tenant_a,
                submission_id,
                TraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .expect("get tenant A active object ref after invalidation")
            .is_none()
    );

    let tenant_a_object_refs = backend
        .list_trace_object_refs(&tenant_a, submission_id)
        .await
        .expect("list tenant A object refs after invalidation");
    assert_eq!(tenant_a_object_refs.len(), 2);
    assert!(
        tenant_a_object_refs
            .iter()
            .all(|object_ref| object_ref.invalidated_at.is_some())
    );
    assert!(
        tenant_a_object_refs
            .iter()
            .all(|object_ref| object_ref.deleted_at.is_none())
    );
    let deleted_count = backend
        .mark_trace_object_ref_deleted(
            &tenant_a,
            submission_id,
            "s3://private-corpus",
            &format!("{tenant_a}/submission.json"),
        )
        .await
        .expect("mark tenant A exact object ref deleted");
    assert_eq!(deleted_count, 1);
    let tenant_a_object_refs_after_delete = backend
        .list_trace_object_refs(&tenant_a, submission_id)
        .await
        .expect("list tenant A object refs after exact delete");
    let deleted_ref = tenant_a_object_refs_after_delete
        .iter()
        .find(|object_ref| object_ref.object_ref_id == tenant_a_first_object_ref_id)
        .expect("tenant A deleted object ref remains listed");
    assert!(deleted_ref.deleted_at.is_some());
    let untouched_ref = tenant_a_object_refs_after_delete
        .iter()
        .find(|object_ref| object_ref.object_ref_id == tenant_a_latest_object_ref_id)
        .expect("tenant A untouched object ref remains listed");
    assert!(untouched_ref.deleted_at.is_none());
    let idempotent_delete = backend
        .mark_trace_object_ref_deleted(
            &tenant_a,
            submission_id,
            "s3://private-corpus",
            &format!("{tenant_a}/submission.json"),
        )
        .await
        .expect("repeat tenant A exact object ref delete");
    assert_eq!(idempotent_delete, 0);

    let tenant_b_still_active = backend
        .get_latest_active_trace_object_ref(
            &tenant_b,
            submission_id,
            TraceObjectArtifactKind::SubmittedEnvelope,
        )
        .await
        .expect("get tenant B active object ref after tenant A invalidation")
        .expect("tenant B object ref remains active");
    assert_eq!(tenant_b_still_active.object_ref_id, tenant_b_object_ref_id);

    let tenant_a_records = backend
        .list_trace_derived_records(&tenant_a)
        .await
        .expect("list tenant A derived records");
    assert_eq!(tenant_a_records.len(), 1);
    assert_eq!(tenant_a_records[0].status, TraceDerivedStatus::Revoked);

    let tenant_b_records = backend
        .list_trace_derived_records(&tenant_b)
        .await
        .expect("list tenant B derived records");
    assert_eq!(tenant_b_records.len(), 1);
    assert_eq!(tenant_b_records[0].status, TraceDerivedStatus::Current);

    let effective_at = DateTime::parse_from_rfc3339("2026-04-25T12:00:00Z")
        .expect("parse effective timestamp")
        .with_timezone(&Utc);
    let retain_until = DateTime::parse_from_rfc3339("2026-05-25T12:00:00Z")
        .expect("parse retain-until timestamp")
        .with_timezone(&Utc);
    let tombstone_id = Uuid::new_v4();
    backend
        .write_trace_tombstone(TraceTombstoneWrite {
            tombstone_id,
            tenant_id: tenant_a.clone(),
            submission_id,
            trace_id: Some(inserted_a.trace_id),
            redaction_hash: Some(format!("sha256:{tenant_a}:redaction")),
            canonical_summary_hash: Some(format!("sha256:{tenant_a}:summary")),
            reason: "user requested revocation".to_string(),
            effective_at,
            retain_until: Some(retain_until),
            created_by_principal_ref: format!("principal:{tenant_a}"),
        })
        .await
        .expect("write tenant A tombstone");

    backend
        .write_trace_tombstone(TraceTombstoneWrite {
            tombstone_id: Uuid::new_v4(),
            tenant_id: tenant_a.clone(),
            submission_id,
            trace_id: Some(inserted_a.trace_id),
            redaction_hash: Some(format!("sha256:{tenant_a}:later-redaction")),
            canonical_summary_hash: Some(format!("sha256:{tenant_a}:later-summary")),
            reason: "later duplicate revocation".to_string(),
            effective_at: Utc::now(),
            retain_until: None,
            created_by_principal_ref: format!("principal:{tenant_a}:later"),
        })
        .await
        .expect("repeat tenant A tombstone write is idempotent");

    backend
        .write_trace_tombstone(TraceTombstoneWrite {
            tombstone_id: Uuid::new_v4(),
            tenant_id: tenant_b.clone(),
            submission_id,
            trace_id: Some(inserted_b.trace_id),
            redaction_hash: Some(format!("sha256:{tenant_b}:redaction")),
            canonical_summary_hash: Some(format!("sha256:{tenant_b}:summary")),
            reason: "other tenant revocation".to_string(),
            effective_at,
            retain_until: None,
            created_by_principal_ref: format!("principal:{tenant_b}"),
        })
        .await
        .expect("write tenant B tombstone");

    let tenant_a_tombstones = backend
        .list_trace_tombstones(&tenant_a)
        .await
        .expect("list tenant A tombstones");
    assert_eq!(tenant_a_tombstones.len(), 1);
    assert_eq!(tenant_a_tombstones[0].tombstone_id, tombstone_id);
    assert_eq!(tenant_a_tombstones[0].tenant_id, tenant_a);
    assert_eq!(tenant_a_tombstones[0].trace_id, Some(inserted_a.trace_id));
    assert_eq!(tenant_a_tombstones[0].reason, "user requested revocation");
    assert_eq!(tenant_a_tombstones[0].retain_until, Some(retain_until));

    let tenant_b_tombstones = backend
        .list_trace_tombstones(&tenant_b)
        .await
        .expect("list tenant B tombstones");
    assert_eq!(tenant_b_tombstones.len(), 1);
    assert_eq!(tenant_b_tombstones[0].tenant_id, tenant_b);
    assert_eq!(tenant_b_tombstones[0].trace_id, Some(inserted_b.trace_id));
    assert_eq!(tenant_b_tombstones[0].reason, "other tenant revocation");

    let mut client = backend.pool().get().await.expect("get cleanup connection");
    for tenant_id in [&tenant_a, &tenant_b] {
        let tx = client
            .transaction()
            .await
            .expect("start cleanup transaction");
        tx.execute(
            "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
            &[tenant_id],
        )
        .await
        .expect("set cleanup tenant context");
        let _ = tx
            .execute(
                "DELETE FROM trace_tenants WHERE tenant_id = $1",
                &[tenant_id],
            )
            .await;
        tx.commit().await.expect("commit cleanup transaction");
    }
}

#[tokio::test]
async fn store_facade_invalidates_export_manifests_by_submission_with_tenant_scope() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");
    assert_trace_rls_policies_installed(&backend).await;

    let submission_id = Uuid::new_v4();
    backend
        .upsert_trace_submission(sample_submission("tenant-alpha", submission_id))
        .await
        .expect("insert alpha submission");
    backend
        .upsert_trace_submission(sample_submission("tenant-beta", submission_id))
        .await
        .expect("insert beta submission");

    let alpha_export_id = Uuid::new_v4();
    let beta_export_id = Uuid::new_v4();
    backend
        .upsert_trace_export_manifest(TraceExportManifestWrite {
            tenant_id: "tenant-alpha".to_string(),
            export_manifest_id: alpha_export_id,
            artifact_kind: TraceObjectArtifactKind::ExportArtifact,
            purpose_code: Some("ranking_dataset".to_string()),
            audit_event_id: Some(Uuid::new_v4()),
            source_submission_ids: vec![submission_id],
            source_submission_ids_hash: "sha256:alpha-sources".to_string(),
            item_count: 1,
            generated_at: Utc::now(),
        })
        .await
        .expect("insert alpha export manifest");
    backend
        .upsert_trace_export_manifest(TraceExportManifestWrite {
            tenant_id: "tenant-beta".to_string(),
            export_manifest_id: beta_export_id,
            artifact_kind: TraceObjectArtifactKind::ExportArtifact,
            purpose_code: Some("ranking_dataset".to_string()),
            audit_event_id: Some(Uuid::new_v4()),
            source_submission_ids: vec![submission_id],
            source_submission_ids_hash: "sha256:beta-sources".to_string(),
            item_count: 1,
            generated_at: Utc::now(),
        })
        .await
        .expect("insert beta export manifest");

    let invalidated = backend
        .invalidate_trace_export_manifests_for_submission("tenant-alpha", submission_id)
        .await
        .expect("invalidate alpha export manifest");
    assert_eq!(invalidated, 1);

    let idempotent = backend
        .invalidate_trace_export_manifests_for_submission("tenant-alpha", submission_id)
        .await
        .expect("repeat export manifest invalidation");
    assert_eq!(idempotent, 0);

    let alpha_manifests = backend
        .list_trace_export_manifests("tenant-alpha")
        .await
        .expect("list alpha export manifests");
    let alpha_manifest = alpha_manifests
        .iter()
        .find(|manifest| manifest.export_manifest_id == alpha_export_id)
        .expect("alpha export manifest exists");
    assert!(alpha_manifest.invalidated_at.is_some());
    assert!(alpha_manifest.deleted_at.is_none());

    let beta_manifests = backend
        .list_trace_export_manifests("tenant-beta")
        .await
        .expect("list beta export manifests");
    let beta_manifest = beta_manifests
        .iter()
        .find(|manifest| manifest.export_manifest_id == beta_export_id)
        .expect("beta export manifest exists");
    assert!(beta_manifest.invalidated_at.is_none());
    assert!(beta_manifest.deleted_at.is_none());

    let mut client = backend.pool().get().await.expect("get cleanup connection");
    for (tenant_id, export_manifest_id) in [
        ("tenant-alpha", alpha_export_id),
        ("tenant-beta", beta_export_id),
    ] {
        let tx = client
            .transaction()
            .await
            .expect("start cleanup transaction");
        tx.execute(
            "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await
        .expect("set cleanup tenant context");
        let _ = tx
            .execute(
                "DELETE FROM trace_export_manifests
                 WHERE tenant_id = $1 AND export_manifest_id = $2",
                &[&tenant_id, &export_manifest_id],
            )
            .await;
        let _ = tx
            .execute(
                "DELETE FROM trace_submissions
                 WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await;
        tx.commit().await.expect("commit cleanup transaction");
    }
}

#[tokio::test]
async fn store_facade_invalidates_export_manifest_items_by_submission_with_tenant_scope() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");
    assert_trace_rls_policies_installed(&backend).await;

    let tenant_a = format!("rls-export-items-a-{}", Uuid::new_v4());
    let tenant_b = format!("rls-export-items-b-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();

    let mut tenant_a_submission = sample_submission(&tenant_a, submission_id);
    tenant_a_submission.trace_id = trace_id;
    backend
        .upsert_trace_submission(tenant_a_submission)
        .await
        .expect("insert tenant A submission");
    let mut tenant_b_submission = sample_submission(&tenant_b, submission_id);
    tenant_b_submission.trace_id = trace_id;
    backend
        .upsert_trace_submission(tenant_b_submission)
        .await
        .expect("insert tenant B submission");

    let tenant_a_export_id = Uuid::new_v4();
    let tenant_b_export_id = Uuid::new_v4();
    backend
        .upsert_trace_export_manifest(TraceExportManifestWrite {
            tenant_id: tenant_a.clone(),
            export_manifest_id: tenant_a_export_id,
            artifact_kind: TraceObjectArtifactKind::ExportArtifact,
            purpose_code: Some("replay_dataset".to_string()),
            audit_event_id: Some(Uuid::new_v4()),
            source_submission_ids: vec![submission_id],
            source_submission_ids_hash: format!("sha256:{tenant_a}:sources"),
            item_count: 1,
            generated_at: Utc::now(),
        })
        .await
        .expect("insert tenant A manifest");
    backend
        .upsert_trace_export_manifest(TraceExportManifestWrite {
            tenant_id: tenant_b.clone(),
            export_manifest_id: tenant_b_export_id,
            artifact_kind: TraceObjectArtifactKind::ExportArtifact,
            purpose_code: Some("replay_dataset".to_string()),
            audit_event_id: Some(Uuid::new_v4()),
            source_submission_ids: vec![submission_id],
            source_submission_ids_hash: format!("sha256:{tenant_b}:sources"),
            item_count: 1,
            generated_at: Utc::now(),
        })
        .await
        .expect("insert tenant B manifest");

    let tenant_a_object_ref_id = Uuid::new_v4();
    let tenant_a_derived_id = Uuid::new_v4();
    let tenant_a_vector_entry_id = Uuid::new_v4();
    backend
        .append_trace_object_ref(TraceObjectRefWrite {
            tenant_id: tenant_a.clone(),
            object_ref_id: tenant_a_object_ref_id,
            submission_id,
            artifact_kind: TraceObjectArtifactKind::WorkerIntermediate,
            object_store: "s3://private-corpus".to_string(),
            object_key: format!("{tenant_a}/worker/summary.json"),
            content_sha256: format!("sha256:{tenant_a}:object"),
            encryption_key_ref: format!("kms:{tenant_a}"),
            size_bytes: 128,
            compression: None,
            created_by_job_id: None,
        })
        .await
        .expect("insert tenant A object ref");
    backend
        .append_trace_derived_record(TraceDerivedRecordWrite {
            tenant_id: tenant_a.clone(),
            derived_id: tenant_a_derived_id,
            submission_id,
            trace_id,
            status: TraceDerivedStatus::Current,
            worker_kind: TraceWorkerKind::Summary,
            worker_version: "summary-worker-v1".to_string(),
            input_object_ref: Some(TenantScopedTraceObjectRef {
                tenant_id: tenant_a.clone(),
                submission_id,
                object_ref_id: tenant_a_object_ref_id,
            }),
            input_hash: format!("sha256:{tenant_a}:object"),
            output_object_ref: None,
            canonical_summary: Some("Tenant A summary.".to_string()),
            canonical_summary_hash: Some(format!("sha256:{tenant_a}:summary")),
            summary_model: "summary-model-v1".to_string(),
            task_success: Some("success".to_string()),
            privacy_risk: Some("low".to_string()),
            event_count: Some(2),
            tool_sequence: vec!["memory_search".to_string()],
            tool_categories: vec!["memory".to_string()],
            coverage_tags: vec!["tool:memory_search".to_string()],
            duplicate_score: Some(0.1),
            novelty_score: Some(0.4),
            cluster_id: Some(format!("cluster:{tenant_a}")),
        })
        .await
        .expect("insert tenant A derived record");
    backend
        .upsert_trace_vector_entry(TraceVectorEntryWrite {
            tenant_id: tenant_a.clone(),
            submission_id,
            derived_id: tenant_a_derived_id,
            vector_entry_id: tenant_a_vector_entry_id,
            vector_store: "trace-commons-main".to_string(),
            embedding_model: "text-embedding-3-small".to_string(),
            embedding_dimension: 1536,
            embedding_version: "embedding-v1".to_string(),
            source_projection: TraceVectorEntrySourceProjection::CanonicalSummary,
            source_hash: format!("sha256:{tenant_a}:summary"),
            status: TraceVectorEntryStatus::Active,
            nearest_trace_ids: Vec::new(),
            cluster_id: Some(format!("cluster:{tenant_a}")),
            duplicate_score: Some(0.1),
            novelty_score: Some(0.4),
            indexed_at: Some(Utc::now()),
            invalidated_at: None,
            deleted_at: None,
        })
        .await
        .expect("insert tenant A vector entry");

    backend
        .upsert_trace_export_manifest_item(TraceExportManifestItemWrite {
            tenant_id: tenant_a.clone(),
            export_manifest_id: tenant_a_export_id,
            submission_id,
            trace_id,
            derived_id: Some(tenant_a_derived_id),
            object_ref_id: Some(tenant_a_object_ref_id),
            vector_entry_id: Some(tenant_a_vector_entry_id),
            source_status_at_export: TraceCorpusStatus::Accepted,
            source_hash_at_export: format!("sha256:{tenant_a}:source"),
        })
        .await
        .expect("insert tenant A manifest item");
    backend
        .upsert_trace_export_manifest_item(TraceExportManifestItemWrite {
            tenant_id: tenant_b.clone(),
            export_manifest_id: tenant_b_export_id,
            submission_id,
            trace_id,
            derived_id: None,
            object_ref_id: None,
            vector_entry_id: None,
            source_status_at_export: TraceCorpusStatus::Accepted,
            source_hash_at_export: format!("sha256:{tenant_b}:source"),
        })
        .await
        .expect("insert tenant B manifest item");

    let tenant_a_items = backend
        .list_trace_export_manifest_items(&tenant_a, tenant_a_export_id)
        .await
        .expect("list tenant A manifest items");
    assert_eq!(tenant_a_items.len(), 1);
    assert_eq!(tenant_a_items[0].tenant_id, tenant_a);
    assert_eq!(tenant_a_items[0].export_manifest_id, tenant_a_export_id);
    assert_eq!(tenant_a_items[0].submission_id, submission_id);
    assert_eq!(tenant_a_items[0].trace_id, trace_id);
    assert_eq!(
        tenant_a_items[0].source_status_at_export,
        TraceCorpusStatus::Accepted
    );
    assert_eq!(
        tenant_a_items[0].source_hash_at_export,
        format!("sha256:{tenant_a}:source")
    );
    assert!(tenant_a_items[0].derived_id.is_some());
    assert!(tenant_a_items[0].object_ref_id.is_some());
    assert!(tenant_a_items[0].vector_entry_id.is_some());
    assert!(tenant_a_items[0].source_invalidated_at.is_none());
    assert!(tenant_a_items[0].source_invalidation_reason.is_none());

    let invalidated = backend
        .invalidate_trace_export_manifest_items_for_submission(
            &tenant_a,
            submission_id,
            TraceExportManifestItemInvalidationReason::Revoked,
        )
        .await
        .expect("invalidate tenant A manifest item");
    assert_eq!(invalidated, 1);
    let idempotent = backend
        .invalidate_trace_export_manifest_items_for_submission(
            &tenant_a,
            submission_id,
            TraceExportManifestItemInvalidationReason::Revoked,
        )
        .await
        .expect("repeat tenant A manifest item invalidation");
    assert_eq!(idempotent, 0);

    let tenant_a_items = backend
        .list_trace_export_manifest_items(&tenant_a, tenant_a_export_id)
        .await
        .expect("list invalidated tenant A manifest items");
    assert!(tenant_a_items[0].source_invalidated_at.is_some());
    assert_eq!(
        tenant_a_items[0].source_invalidation_reason,
        Some(TraceExportManifestItemInvalidationReason::Revoked)
    );

    let tenant_b_items = backend
        .list_trace_export_manifest_items(&tenant_b, tenant_b_export_id)
        .await
        .expect("list tenant B manifest items");
    assert_eq!(tenant_b_items.len(), 1);
    assert_eq!(tenant_b_items[0].tenant_id, tenant_b);
    assert_eq!(tenant_b_items[0].submission_id, submission_id);
    assert!(tenant_b_items[0].source_invalidated_at.is_none());
    assert!(tenant_b_items[0].source_invalidation_reason.is_none());

    let mut client = backend.pool().get().await.expect("get cleanup connection");
    for tenant_id in [&tenant_a, &tenant_b] {
        let tx = client
            .transaction()
            .await
            .expect("start cleanup transaction");
        tx.execute(
            "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
            &[tenant_id],
        )
        .await
        .expect("set cleanup tenant context");
        let _ = tx
            .execute(
                "DELETE FROM trace_tenants WHERE tenant_id = $1",
                &[tenant_id],
            )
            .await;
        tx.commit().await.expect("commit cleanup transaction");
    }
}

#[tokio::test]
async fn store_facade_rejects_export_manifest_item_cross_tenant_refs() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");
    assert_trace_rls_policies_installed(&backend).await;

    let tenant_a = format!("rls-export-ref-a-{}", Uuid::new_v4());
    let tenant_b = format!("rls-export-ref-b-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();

    let mut tenant_a_submission = sample_submission(&tenant_a, submission_id);
    tenant_a_submission.trace_id = trace_id;
    backend
        .upsert_trace_submission(tenant_a_submission)
        .await
        .expect("insert tenant A submission");
    let mut tenant_b_submission = sample_submission(&tenant_b, submission_id);
    tenant_b_submission.trace_id = trace_id;
    backend
        .upsert_trace_submission(tenant_b_submission)
        .await
        .expect("insert tenant B submission");

    let tenant_b_object_ref_id = Uuid::new_v4();
    let tenant_b_derived_id = Uuid::new_v4();
    let tenant_b_vector_entry_id = Uuid::new_v4();
    backend
        .append_trace_object_ref(TraceObjectRefWrite {
            tenant_id: tenant_b.clone(),
            object_ref_id: tenant_b_object_ref_id,
            submission_id,
            artifact_kind: TraceObjectArtifactKind::WorkerIntermediate,
            object_store: "s3://private-corpus".to_string(),
            object_key: format!("{tenant_b}/worker/summary.json"),
            content_sha256: format!("sha256:{tenant_b}:object"),
            encryption_key_ref: format!("kms:{tenant_b}"),
            size_bytes: 128,
            compression: None,
            created_by_job_id: None,
        })
        .await
        .expect("insert tenant B object ref");
    backend
        .append_trace_derived_record(TraceDerivedRecordWrite {
            tenant_id: tenant_b.clone(),
            derived_id: tenant_b_derived_id,
            submission_id,
            trace_id,
            status: TraceDerivedStatus::Current,
            worker_kind: TraceWorkerKind::Summary,
            worker_version: "summary-worker-v1".to_string(),
            input_object_ref: Some(TenantScopedTraceObjectRef {
                tenant_id: tenant_b.clone(),
                submission_id,
                object_ref_id: tenant_b_object_ref_id,
            }),
            input_hash: format!("sha256:{tenant_b}:object"),
            output_object_ref: None,
            canonical_summary: Some("Tenant B summary.".to_string()),
            canonical_summary_hash: Some(format!("sha256:{tenant_b}:summary")),
            summary_model: "summary-model-v1".to_string(),
            task_success: Some("success".to_string()),
            privacy_risk: Some("low".to_string()),
            event_count: Some(2),
            tool_sequence: vec!["memory_search".to_string()],
            tool_categories: vec!["memory".to_string()],
            coverage_tags: vec!["tool:memory_search".to_string()],
            duplicate_score: Some(0.1),
            novelty_score: Some(0.4),
            cluster_id: Some(format!("cluster:{tenant_b}")),
        })
        .await
        .expect("insert tenant B derived record");
    backend
        .upsert_trace_vector_entry(TraceVectorEntryWrite {
            tenant_id: tenant_b.clone(),
            submission_id,
            derived_id: tenant_b_derived_id,
            vector_entry_id: tenant_b_vector_entry_id,
            vector_store: "trace-commons-main".to_string(),
            embedding_model: "text-embedding-3-small".to_string(),
            embedding_dimension: 1536,
            embedding_version: "embedding-v1".to_string(),
            source_projection: TraceVectorEntrySourceProjection::CanonicalSummary,
            source_hash: format!("sha256:{tenant_b}:summary"),
            status: TraceVectorEntryStatus::Active,
            nearest_trace_ids: Vec::new(),
            cluster_id: Some(format!("cluster:{tenant_b}")),
            duplicate_score: Some(0.1),
            novelty_score: Some(0.4),
            indexed_at: Some(Utc::now()),
            invalidated_at: None,
            deleted_at: None,
        })
        .await
        .expect("insert tenant B vector entry");

    let tenant_a_export_id = Uuid::new_v4();
    backend
        .upsert_trace_export_manifest(TraceExportManifestWrite {
            tenant_id: tenant_a.clone(),
            export_manifest_id: tenant_a_export_id,
            artifact_kind: TraceObjectArtifactKind::ExportArtifact,
            purpose_code: Some("replay_dataset".to_string()),
            audit_event_id: Some(Uuid::new_v4()),
            source_submission_ids: vec![submission_id],
            source_submission_ids_hash: format!("sha256:{tenant_a}:sources"),
            item_count: 1,
            generated_at: Utc::now(),
        })
        .await
        .expect("insert tenant A manifest");

    let err = backend
        .upsert_trace_export_manifest_item(TraceExportManifestItemWrite {
            tenant_id: tenant_a.clone(),
            export_manifest_id: tenant_a_export_id,
            submission_id,
            trace_id,
            derived_id: Some(tenant_b_derived_id),
            object_ref_id: Some(tenant_b_object_ref_id),
            vector_entry_id: Some(tenant_b_vector_entry_id),
            source_status_at_export: TraceCorpusStatus::Accepted,
            source_hash_at_export: format!("sha256:{tenant_a}:source"),
        })
        .await
        .expect_err("cross-tenant export refs must be rejected");

    assert!(
        err.to_string().contains("does not belong to tenant"),
        "unexpected error: {err}"
    );

    let mut client = backend.pool().get().await.expect("get cleanup connection");
    for tenant_id in [&tenant_a, &tenant_b] {
        let tx = client
            .transaction()
            .await
            .expect("start cleanup transaction");
        tx.execute(
            "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
            &[tenant_id],
        )
        .await
        .expect("set cleanup tenant context");
        let _ = tx
            .execute(
                "DELETE FROM trace_tenants WHERE tenant_id = $1",
                &[tenant_id],
            )
            .await;
        tx.commit().await.expect("commit cleanup transaction");
    }
}

#[tokio::test]
async fn store_facade_rejects_derived_record_mismatched_tenant_object_ref() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");
    assert_trace_rls_policies_installed(&backend).await;

    let tenant_a = format!("rls-derived-ref-a-{}", Uuid::new_v4());
    let tenant_b = format!("rls-derived-ref-b-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();

    let mut tenant_a_submission = sample_submission(&tenant_a, submission_id);
    tenant_a_submission.trace_id = trace_id;
    backend
        .upsert_trace_submission(tenant_a_submission)
        .await
        .expect("insert tenant A submission");
    let mut tenant_b_submission = sample_submission(&tenant_b, submission_id);
    tenant_b_submission.trace_id = trace_id;
    backend
        .upsert_trace_submission(tenant_b_submission)
        .await
        .expect("insert tenant B submission");

    let tenant_b_object_ref_id = Uuid::new_v4();
    backend
        .append_trace_object_ref(TraceObjectRefWrite {
            tenant_id: tenant_b.clone(),
            object_ref_id: tenant_b_object_ref_id,
            submission_id,
            artifact_kind: TraceObjectArtifactKind::WorkerIntermediate,
            object_store: "s3://private-corpus".to_string(),
            object_key: format!("{tenant_b}/worker/summary.json"),
            content_sha256: format!("sha256:{tenant_b}:object"),
            encryption_key_ref: format!("kms:{tenant_b}"),
            size_bytes: 128,
            compression: None,
            created_by_job_id: None,
        })
        .await
        .expect("insert tenant B object ref");

    let err = backend
        .append_trace_derived_record(TraceDerivedRecordWrite {
            tenant_id: tenant_a.clone(),
            derived_id: Uuid::new_v4(),
            submission_id,
            trace_id,
            status: TraceDerivedStatus::Current,
            worker_kind: TraceWorkerKind::Summary,
            worker_version: "summary-worker-v1".to_string(),
            input_object_ref: Some(TenantScopedTraceObjectRef {
                tenant_id: tenant_b.clone(),
                submission_id,
                object_ref_id: tenant_b_object_ref_id,
            }),
            input_hash: format!("sha256:{tenant_b}:object"),
            output_object_ref: None,
            canonical_summary: Some("Tenant A summary.".to_string()),
            canonical_summary_hash: Some(format!("sha256:{tenant_a}:summary")),
            summary_model: "summary-model-v1".to_string(),
            task_success: Some("success".to_string()),
            privacy_risk: Some("low".to_string()),
            event_count: Some(2),
            tool_sequence: vec!["memory_search".to_string()],
            tool_categories: vec!["memory".to_string()],
            coverage_tags: vec!["tool:memory_search".to_string()],
            duplicate_score: Some(0.1),
            novelty_score: Some(0.4),
            cluster_id: Some(format!("cluster:{tenant_a}")),
        })
        .await
        .expect_err("derived records must reject cross-tenant object refs");

    assert!(
        err.to_string().contains("does not belong to tenant"),
        "unexpected error: {err}"
    );

    let mut client = backend.pool().get().await.expect("get cleanup connection");
    for tenant_id in [&tenant_a, &tenant_b] {
        let tx = client
            .transaction()
            .await
            .expect("start cleanup transaction");
        tx.execute(
            "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
            &[tenant_id],
        )
        .await
        .expect("set cleanup tenant context");
        let _ = tx
            .execute(
                "DELETE FROM trace_tenants WHERE tenant_id = $1",
                &[tenant_id],
            )
            .await;
        tx.commit().await.expect("commit cleanup transaction");
    }
}

#[tokio::test]
async fn store_facade_rejects_vector_entry_mismatched_submission_derived_id() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");
    assert_trace_rls_policies_installed(&backend).await;

    let tenant_id = format!("rls-vector-derived-{}", Uuid::new_v4());
    let submission_a_id = Uuid::new_v4();
    let trace_a_id = Uuid::new_v4();
    let mut submission_a = sample_submission(&tenant_id, submission_a_id);
    submission_a.trace_id = trace_a_id;
    backend
        .upsert_trace_submission(submission_a)
        .await
        .expect("insert submission A");

    let submission_b_id = Uuid::new_v4();
    let trace_b_id = Uuid::new_v4();
    let mut submission_b = sample_submission(&tenant_id, submission_b_id);
    submission_b.trace_id = trace_b_id;
    backend
        .upsert_trace_submission(submission_b)
        .await
        .expect("insert submission B");

    let object_ref_b_id = Uuid::new_v4();
    let derived_b_id = Uuid::new_v4();
    backend
        .append_trace_object_ref(TraceObjectRefWrite {
            tenant_id: tenant_id.clone(),
            object_ref_id: object_ref_b_id,
            submission_id: submission_b_id,
            artifact_kind: TraceObjectArtifactKind::WorkerIntermediate,
            object_store: "s3://private-corpus".to_string(),
            object_key: format!("{tenant_id}/submission-b/summary.json"),
            content_sha256: format!("sha256:{tenant_id}:submission-b-object"),
            encryption_key_ref: format!("kms:{tenant_id}"),
            size_bytes: 128,
            compression: None,
            created_by_job_id: None,
        })
        .await
        .expect("insert submission B object ref");
    backend
        .append_trace_derived_record(TraceDerivedRecordWrite {
            tenant_id: tenant_id.clone(),
            derived_id: derived_b_id,
            submission_id: submission_b_id,
            trace_id: trace_b_id,
            status: TraceDerivedStatus::Current,
            worker_kind: TraceWorkerKind::Summary,
            worker_version: "summary-worker-v1".to_string(),
            input_object_ref: Some(TenantScopedTraceObjectRef {
                tenant_id: tenant_id.clone(),
                submission_id: submission_b_id,
                object_ref_id: object_ref_b_id,
            }),
            input_hash: format!("sha256:{tenant_id}:submission-b-object"),
            output_object_ref: None,
            canonical_summary: Some("Submission B summary.".to_string()),
            canonical_summary_hash: Some(format!("sha256:{tenant_id}:submission-b-summary")),
            summary_model: "summary-model-v1".to_string(),
            task_success: Some("success".to_string()),
            privacy_risk: Some("low".to_string()),
            event_count: Some(2),
            tool_sequence: vec!["memory_search".to_string()],
            tool_categories: vec!["memory".to_string()],
            coverage_tags: vec!["tool:memory_search".to_string()],
            duplicate_score: Some(0.1),
            novelty_score: Some(0.4),
            cluster_id: Some(format!("cluster:{tenant_id}")),
        })
        .await
        .expect("insert submission B derived record");

    let err = backend
        .upsert_trace_vector_entry(TraceVectorEntryWrite {
            tenant_id: tenant_id.clone(),
            submission_id: submission_a_id,
            derived_id: derived_b_id,
            vector_entry_id: Uuid::new_v4(),
            vector_store: "trace-commons-main".to_string(),
            embedding_model: "text-embedding-3-small".to_string(),
            embedding_dimension: 1536,
            embedding_version: "embedding-v1".to_string(),
            source_projection: TraceVectorEntrySourceProjection::CanonicalSummary,
            source_hash: format!("sha256:{tenant_id}:submission-a-summary"),
            status: TraceVectorEntryStatus::Active,
            nearest_trace_ids: Vec::new(),
            cluster_id: Some(format!("cluster:{tenant_id}")),
            duplicate_score: Some(0.1),
            novelty_score: Some(0.4),
            indexed_at: Some(Utc::now()),
            invalidated_at: None,
            deleted_at: None,
        })
        .await
        .expect_err("vector entries must reject derived ids from another submission");

    assert!(
        err.to_string().contains("does not belong to tenant"),
        "unexpected error: {err}"
    );

    let mut client = backend.pool().get().await.expect("get cleanup connection");
    let tx = client
        .transaction()
        .await
        .expect("start cleanup transaction");
    tx.execute(
        "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set cleanup tenant context");
    let _ = tx
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await;
    tx.commit().await.expect("commit cleanup transaction");
}
