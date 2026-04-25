#![cfg(feature = "postgres")]

use std::collections::BTreeMap;

use ironclaw::config::{DatabaseBackend, DatabaseConfig, SslMode};
use ironclaw::db::{Database, postgres::PgBackend};
use ironclaw::trace_corpus_storage::{TraceCorpusStatus, TraceCorpusStore, TraceSubmissionWrite};
use secrecy::{ExposeSecret, SecretString};
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

    let client = backend.pool().get().await.expect("get cleanup connection");
    let _ = client
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = ANY($1)",
            &[&vec![tenant_a, tenant_b]],
        )
        .await;
}
