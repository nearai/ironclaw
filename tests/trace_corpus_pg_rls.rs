#![cfg(feature = "postgres")]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use ironclaw::config::{DatabaseBackend, DatabaseConfig, SslMode};
use ironclaw::db::{Database, postgres::PgBackend};
use ironclaw::trace_corpus_storage::{
    TenantScopedTraceObjectRef, TraceCorpusStatus, TraceCorpusStore, TraceDerivedRecordWrite,
    TraceDerivedStatus, TraceExportManifestItemInvalidationReason, TraceExportManifestItemWrite,
    TraceExportManifestWrite, TraceObjectArtifactKind, TraceObjectRefWrite, TraceSubmissionWrite,
    TraceTombstoneWrite, TraceWorkerKind,
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

    backend
        .upsert_trace_export_manifest_item(TraceExportManifestItemWrite {
            tenant_id: tenant_a.clone(),
            export_manifest_id: tenant_a_export_id,
            submission_id,
            trace_id,
            derived_id: Some(Uuid::new_v4()),
            object_ref_id: Some(Uuid::new_v4()),
            vector_entry_id: Some(Uuid::new_v4()),
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
