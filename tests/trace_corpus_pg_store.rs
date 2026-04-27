#![cfg(feature = "postgres")]

use std::collections::BTreeMap;

use chrono::Utc;
use ironclaw::config::{DatabaseBackend, DatabaseConfig, SslMode};
use ironclaw::db::{Database, postgres::PgBackend};
use ironclaw::error::DatabaseError;
use ironclaw::trace_corpus_storage::{
    TraceCorpusStatus, TraceCorpusStore, TraceDerivedRecordWrite, TraceDerivedStatus,
    TraceExportManifestItemWrite, TraceExportManifestMirrorWrite, TraceExportManifestWrite,
    TraceObjectArtifactKind, TraceObjectRefWrite, TraceSubmissionWrite, TraceWorkerKind,
};
use secrecy::SecretString;
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

fn sample_submission(tenant_id: &str, submission_id: Uuid) -> TraceSubmissionWrite {
    let mut redaction_counts = BTreeMap::new();
    redaction_counts.insert("secret".to_string(), 2);
    redaction_counts.insert("private_email".to_string(), 1);

    TraceSubmissionWrite {
        tenant_id: tenant_id.to_string(),
        submission_id,
        trace_id: Uuid::new_v4(),
        auth_principal_ref: "principal:test-user".to_string(),
        contributor_pseudonym: Some("contributor:test".to_string()),
        submitted_tenant_scope_ref: Some(tenant_id.to_string()),
        schema_version: "ironclaw.trace_contribution.v1".to_string(),
        consent_policy_version: "2026-04-24".to_string(),
        consent_scopes: vec!["training_allowed".to_string()],
        allowed_uses: vec!["debugging".to_string(), "training".to_string()],
        retention_policy_id: "standard".to_string(),
        status: TraceCorpusStatus::Accepted,
        privacy_risk: "low".to_string(),
        redaction_pipeline_version: "deterministic-v1".to_string(),
        redaction_counts,
        redaction_hash: "sha256:redaction".to_string(),
        canonical_summary_hash: Some("sha256:canonical".to_string()),
        submission_score: Some(0.82),
        credit_points_pending: Some(1.0),
        credit_points_final: None,
        expires_at: None,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ExportMirrorCounts {
    manifests: i64,
    object_refs: i64,
    items: i64,
}

async fn export_mirror_counts(
    backend: &PgBackend,
    tenant_id: &str,
    export_manifest_id: Uuid,
) -> ExportMirrorCounts {
    let mut client = backend.pool().get().await.expect("get count connection");
    let tx = client.transaction().await.expect("start count transaction");
    tx.execute(
        "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set count tenant context");
    let row = tx
        .query_one(
            "SELECT
                (SELECT COUNT(*) FROM trace_export_manifests
                 WHERE tenant_id = $1 AND export_manifest_id = $2) AS manifests,
                (SELECT COUNT(*) FROM trace_object_refs
                 WHERE tenant_id = $1 AND created_by_job_id = $2) AS object_refs,
                (SELECT COUNT(*) FROM trace_export_manifest_items
                 WHERE tenant_id = $1 AND export_manifest_id = $2) AS items",
            &[&tenant_id, &export_manifest_id],
        )
        .await
        .expect("count export mirror rows");
    tx.commit().await.expect("commit count transaction");

    ExportMirrorCounts {
        manifests: row.get("manifests"),
        object_refs: row.get("object_refs"),
        items: row.get("items"),
    }
}

async fn cleanup_tenant(backend: &PgBackend, tenant_id: &str) {
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

#[tokio::test]
async fn pg_store_rolls_back_export_manifest_mirror_when_item_ref_is_invalid() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let tenant_id = format!("pg-export-mirror-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    let mut submission = sample_submission(&tenant_id, submission_id);
    submission.trace_id = trace_id;
    backend
        .upsert_trace_submission(submission)
        .await
        .expect("insert submission");

    let export_id = Uuid::new_v4();
    let object_ref_id = Uuid::new_v4();
    let derived_id = Uuid::new_v4();
    let missing_derived_id = Uuid::new_v4();
    backend
        .append_trace_derived_record(TraceDerivedRecordWrite {
            tenant_id: tenant_id.clone(),
            derived_id,
            submission_id,
            trace_id,
            status: TraceDerivedStatus::Current,
            worker_kind: TraceWorkerKind::Summary,
            worker_version: "summary-worker-v1".to_string(),
            input_object_ref: None,
            input_hash: "sha256:input".to_string(),
            output_object_ref: None,
            canonical_summary: Some("Tenant alpha summary.".to_string()),
            canonical_summary_hash: Some("sha256:alpha-summary".to_string()),
            summary_model: "summary-model-v1".to_string(),
            task_success: Some("success".to_string()),
            privacy_risk: Some("low".to_string()),
            event_count: Some(2),
            tool_sequence: vec!["memory_search".to_string()],
            tool_categories: vec!["memory".to_string()],
            coverage_tags: vec!["tool:memory_search".to_string()],
            duplicate_score: Some(0.1),
            novelty_score: Some(0.4),
            cluster_id: Some("cluster:alpha".to_string()),
        })
        .await
        .expect("insert valid derived record");

    let error = backend
        .upsert_trace_export_manifest_mirror(TraceExportManifestMirrorWrite {
            manifest: TraceExportManifestWrite {
                tenant_id: tenant_id.clone(),
                export_manifest_id: export_id,
                artifact_kind: TraceObjectArtifactKind::BenchmarkArtifact,
                purpose_code: Some("atomic_mirror_failure".to_string()),
                audit_event_id: Some(Uuid::new_v4()),
                source_submission_ids: vec![submission_id],
                source_submission_ids_hash: "sha256:atomic-sources".to_string(),
                item_count: 2,
                generated_at: Utc::now(),
            },
            object_refs: vec![TraceObjectRefWrite {
                tenant_id: tenant_id.clone(),
                object_ref_id,
                submission_id,
                artifact_kind: TraceObjectArtifactKind::BenchmarkArtifact,
                object_store: "trace_commons_file_store".to_string(),
                object_key: format!("{tenant_id}/benchmarks/export/artifact.json"),
                content_sha256: "sha256:artifact".to_string(),
                encryption_key_ref: format!("tenant:{tenant_id}"),
                size_bytes: 128,
                compression: None,
                created_by_job_id: Some(export_id),
            }],
            items: vec![
                TraceExportManifestItemWrite {
                    tenant_id: tenant_id.clone(),
                    export_manifest_id: export_id,
                    submission_id,
                    trace_id,
                    derived_id: Some(derived_id),
                    object_ref_id: Some(object_ref_id),
                    vector_entry_id: None,
                    source_status_at_export: TraceCorpusStatus::Accepted,
                    source_hash_at_export: "sha256:valid-source".to_string(),
                },
                TraceExportManifestItemWrite {
                    tenant_id: tenant_id.clone(),
                    export_manifest_id: export_id,
                    submission_id,
                    trace_id,
                    derived_id: Some(missing_derived_id),
                    object_ref_id: Some(object_ref_id),
                    vector_entry_id: None,
                    source_status_at_export: TraceCorpusStatus::Accepted,
                    source_hash_at_export: "sha256:invalid-source".to_string(),
                },
            ],
        })
        .await
        .expect_err("invalid item ref rolls back whole export mirror");
    assert!(
        matches!(error, DatabaseError::Constraint(_)),
        "unexpected mirror error: {error}"
    );

    let manifests = backend
        .list_trace_export_manifests(&tenant_id)
        .await
        .expect("list manifests after failed mirror");
    assert!(
        manifests
            .iter()
            .all(|manifest| manifest.export_manifest_id != export_id),
        "failed mirror must roll back staged export manifest"
    );
    let items = backend
        .list_trace_export_manifest_items(&tenant_id, export_id)
        .await
        .expect("list manifest items after failed mirror");
    assert!(items.is_empty());
    let object_refs = backend
        .list_trace_object_refs(&tenant_id, submission_id)
        .await
        .expect("list object refs after failed mirror");
    assert!(
        object_refs
            .iter()
            .all(|object_ref| object_ref.created_by_job_id != Some(export_id)),
        "failed mirror must roll back staged export object refs"
    );
    assert_eq!(
        export_mirror_counts(&backend, &tenant_id, export_id).await,
        ExportMirrorCounts {
            manifests: 0,
            object_refs: 0,
            items: 0,
        }
    );

    cleanup_tenant(&backend, &tenant_id).await;
}
