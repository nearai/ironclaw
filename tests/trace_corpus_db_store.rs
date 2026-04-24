#[cfg(feature = "libsql")]
mod libsql_trace_corpus_store {
    use chrono::Utc;
    use ironclaw::db::{Database, libsql::LibSqlBackend};
    use ironclaw::trace_corpus_storage::{
        TenantScopedTraceObjectRef, TraceAuditAction, TraceAuditEventWrite, TraceAuditSafeMetadata,
        TraceCorpusStatus, TraceCorpusStore, TraceCreditEventType, TraceCreditEventWrite,
        TraceCreditSettlementState, TraceDerivedRecordWrite, TraceDerivedStatus,
        TraceObjectArtifactKind, TraceObjectRefWrite, TraceSubmissionWrite, TraceTombstoneWrite,
        TraceWorkerKind,
    };
    use uuid::Uuid;

    fn sample_submission(tenant_id: &str, submission_id: Uuid) -> TraceSubmissionWrite {
        TraceSubmissionWrite {
            tenant_id: tenant_id.to_string(),
            submission_id,
            trace_id: Uuid::new_v4(),
            auth_principal_ref: "principal:test-user".to_string(),
            contributor_pseudonym: Some("contributor:test".to_string()),
            submitted_tenant_scope_ref: Some(tenant_id.to_string()),
            consent_policy_version: "2026-04-24".to_string(),
            schema_version: "ironclaw.trace_contribution.v1".to_string(),
            consent_scopes: vec!["training_allowed".to_string()],
            allowed_uses: vec!["debugging".to_string(), "training".to_string()],
            retention_policy_id: "standard".to_string(),
            status: TraceCorpusStatus::Accepted,
            privacy_risk: "low".to_string(),
            redaction_pipeline_version: "deterministic-v1".to_string(),
            redaction_hash: "sha256:redaction".to_string(),
            canonical_summary_hash: Some("sha256:canonical".to_string()),
            submission_score: Some(0.82),
            credit_points_pending: Some(1.0),
            credit_points_final: None,
            expires_at: None,
        }
    }

    #[tokio::test]
    async fn libsql_store_preserves_tenant_scope_and_status_lifecycle() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir.path().join("trace-corpus.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");

        let tenant_id = "tenant-alpha";
        let submission_id = Uuid::new_v4();
        let inserted = backend
            .upsert_trace_submission(sample_submission(tenant_id, submission_id))
            .await
            .expect("insert submission");

        assert_eq!(inserted.tenant_id, tenant_id);
        assert_eq!(inserted.submission_id, submission_id);
        assert_eq!(inserted.status, TraceCorpusStatus::Accepted);

        let object_ref_id = Uuid::new_v4();
        backend
            .append_trace_object_ref(TraceObjectRefWrite {
                tenant_id: tenant_id.to_string(),
                object_ref_id,
                submission_id,
                artifact_kind: TraceObjectArtifactKind::SubmittedEnvelope,
                object_store: "s3://private-corpus".to_string(),
                object_key: "tenant-alpha/submission.json".to_string(),
                content_sha256: "sha256:object".to_string(),
                encryption_key_ref: "kms:tenant-alpha".to_string(),
                size_bytes: 4096,
                compression: None,
                created_by_job_id: None,
            })
            .await
            .expect("append object ref");

        backend
            .append_trace_derived_record(TraceDerivedRecordWrite {
                tenant_id: tenant_id.to_string(),
                derived_id: Uuid::new_v4(),
                submission_id,
                trace_id: inserted.trace_id,
                status: TraceDerivedStatus::Current,
                worker_kind: TraceWorkerKind::BenchmarkConversion,
                worker_version: "benchmark-worker-v1".to_string(),
                input_object_ref: Some(TenantScopedTraceObjectRef {
                    tenant_id: tenant_id.to_string(),
                    submission_id,
                    object_ref_id,
                }),
                input_hash: "sha256:object".to_string(),
                output_object_ref: None,
                canonical_summary: Some("Converted into a benchmark candidate.".to_string()),
                canonical_summary_hash: Some("sha256:canonical".to_string()),
                task_success: Some("success".to_string()),
                privacy_risk: Some("low".to_string()),
                event_count: Some(3),
                duplicate_score: Some(0.1),
                novelty_score: Some(0.7),
                cluster_id: Some("cluster:test".to_string()),
            })
            .await
            .expect("append derived record");

        backend
            .append_trace_audit_event(TraceAuditEventWrite {
                tenant_id: tenant_id.to_string(),
                audit_event_id: Uuid::new_v4(),
                submission_id: Some(submission_id),
                actor_principal_ref: "principal:test-user".to_string(),
                actor_role: "contributor".to_string(),
                action: TraceAuditAction::Submit,
                reason: None,
                request_id: Some("request:test".to_string()),
                object_ref_id: None,
                export_manifest_id: None,
                decision_inputs_hash: None,
                metadata: TraceAuditSafeMetadata::Submission {
                    status: TraceCorpusStatus::Accepted,
                    privacy_risk: "low".to_string(),
                },
            })
            .await
            .expect("append audit event");

        backend
            .append_trace_credit_event(TraceCreditEventWrite {
                tenant_id: tenant_id.to_string(),
                credit_event_id: Uuid::new_v4(),
                submission_id,
                trace_id: inserted.trace_id,
                credit_account_ref: "credit:test".to_string(),
                event_type: TraceCreditEventType::Accepted,
                points_delta: "1.0".to_string(),
                reason: "Accepted by privacy checks.".to_string(),
                external_ref: None,
                actor_principal_ref: "principal:test-user".to_string(),
                actor_role: "contributor".to_string(),
                settlement_state: TraceCreditSettlementState::Pending,
            })
            .await
            .expect("append credit event");

        let same_tenant = backend
            .get_trace_submission(tenant_id, submission_id)
            .await
            .expect("get submission");
        assert!(same_tenant.is_some());

        let other_tenant = backend
            .get_trace_submission("tenant-beta", submission_id)
            .await
            .expect("tenant-isolated get");
        assert!(other_tenant.is_none());

        backend
            .update_trace_submission_status(
                tenant_id,
                submission_id,
                TraceCorpusStatus::Revoked,
                "principal:test-user",
                Some("user requested revocation"),
            )
            .await
            .expect("update status");

        backend
            .write_trace_tombstone(TraceTombstoneWrite {
                tombstone_id: Uuid::new_v4(),
                tenant_id: tenant_id.to_string(),
                submission_id,
                trace_id: Some(inserted.trace_id),
                redaction_hash: Some("sha256:redaction".to_string()),
                canonical_summary_hash: Some("sha256:canonical".to_string()),
                reason: "user requested revocation".to_string(),
                effective_at: Utc::now(),
                retain_until: None,
                created_by_principal_ref: "principal:test-user".to_string(),
            })
            .await
            .expect("append tombstone");

        let revoked = backend
            .get_trace_submission(tenant_id, submission_id)
            .await
            .expect("get revoked submission")
            .expect("submission should still have tombstone metadata");
        assert_eq!(revoked.status, TraceCorpusStatus::Revoked);
        assert!(revoked.revoked_at.is_some());
    }
}
