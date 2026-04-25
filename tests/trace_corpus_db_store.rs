#[cfg(feature = "libsql")]
mod libsql_trace_corpus_store {
    use std::collections::BTreeMap;

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
            consent_policy_version: "2026-04-24".to_string(),
            schema_version: "ironclaw.trace_contribution.v1".to_string(),
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
        assert_eq!(
            inserted.submitted_tenant_scope_ref.as_deref(),
            Some(tenant_id)
        );
        assert_eq!(inserted.schema_version, "ironclaw.trace_contribution.v1");
        assert_eq!(inserted.consent_policy_version, "2026-04-24");
        assert_eq!(inserted.consent_scopes, vec!["training_allowed"]);
        assert_eq!(inserted.allowed_uses, vec!["debugging", "training"]);
        assert_eq!(inserted.retention_policy_id, "standard");
        assert_eq!(inserted.privacy_risk, "low");
        assert_eq!(inserted.redaction_pipeline_version, "deterministic-v1");
        assert_eq!(inserted.redaction_counts.get("secret"), Some(&2));
        assert_eq!(inserted.redaction_counts.get("private_email"), Some(&1));
        assert!(
            inserted
                .submission_score
                .is_some_and(|score| (score - 0.82).abs() < 0.001)
        );
        assert!(
            inserted
                .credit_points_pending
                .is_some_and(|points| (points - 1.0).abs() < 0.001)
        );
        assert!(inserted.credit_points_final.is_none());

        let listed = backend
            .list_trace_submissions(tenant_id)
            .await
            .expect("list submissions for tenant");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].submission_id, submission_id);
        assert_eq!(listed[0].privacy_risk, "low");

        let other_tenant_list = backend
            .list_trace_submissions("tenant-beta")
            .await
            .expect("list submissions for other tenant");
        assert!(other_tenant_list.is_empty());

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
                summary_model: "summary-model-v1".to_string(),
                task_success: Some("success".to_string()),
                privacy_risk: Some("low".to_string()),
                event_count: Some(3),
                tool_sequence: vec!["calendar_create".to_string(), "memory_search".to_string()],
                tool_categories: vec!["calendar".to_string(), "memory".to_string()],
                coverage_tags: vec![
                    "tool:calendar_create".to_string(),
                    "privacy:low".to_string(),
                ],
                duplicate_score: Some(0.1),
                novelty_score: Some(0.7),
                cluster_id: Some("cluster:test".to_string()),
            })
            .await
            .expect("append derived record");

        let derived_records = backend
            .list_trace_derived_records(tenant_id)
            .await
            .expect("list derived records for tenant");
        assert_eq!(derived_records.len(), 1);
        assert_eq!(derived_records[0].submission_id, submission_id);
        assert_eq!(derived_records[0].trace_id, inserted.trace_id);
        assert_eq!(derived_records[0].status, TraceDerivedStatus::Current);
        assert_eq!(
            derived_records[0].worker_kind,
            TraceWorkerKind::BenchmarkConversion
        );
        assert_eq!(
            derived_records[0].canonical_summary.as_deref(),
            Some("Converted into a benchmark candidate.")
        );
        assert_eq!(derived_records[0].summary_model, "summary-model-v1");
        assert_eq!(
            derived_records[0].tool_sequence,
            vec!["calendar_create", "memory_search"]
        );
        assert_eq!(
            derived_records[0].tool_categories,
            vec!["calendar", "memory"]
        );
        assert_eq!(
            derived_records[0].coverage_tags,
            vec!["tool:calendar_create", "privacy:low"]
        );
        assert_eq!(derived_records[0].duplicate_score, Some(0.1));
        assert_eq!(derived_records[0].novelty_score, Some(0.7));
        assert_eq!(
            derived_records[0]
                .input_object_ref
                .as_ref()
                .map(|ref_| ref_.object_ref_id),
            Some(object_ref_id)
        );

        let other_tenant_derived_records = backend
            .list_trace_derived_records("tenant-beta")
            .await
            .expect("list derived records for other tenant");
        assert!(other_tenant_derived_records.is_empty());

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

        let credit_events = backend
            .list_trace_credit_events(tenant_id)
            .await
            .expect("list credit events for tenant");
        assert_eq!(credit_events.len(), 1);
        assert_eq!(credit_events[0].submission_id, submission_id);
        assert_eq!(credit_events[0].trace_id, inserted.trace_id);
        assert_eq!(credit_events[0].credit_account_ref, "credit:test");
        assert_eq!(credit_events[0].event_type, TraceCreditEventType::Accepted);
        assert_eq!(credit_events[0].points_delta, "1.0");
        assert_eq!(
            credit_events[0].settlement_state,
            TraceCreditSettlementState::Pending
        );
        assert_eq!(credit_events[0].actor_role, "contributor");

        let other_tenant_credit_events = backend
            .list_trace_credit_events("tenant-beta")
            .await
            .expect("list credit events for other tenant");
        assert!(other_tenant_credit_events.is_empty());

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

        let invalidated = backend
            .invalidate_trace_submission_artifacts(
                tenant_id,
                submission_id,
                TraceDerivedStatus::Revoked,
            )
            .await
            .expect("invalidate submission artifacts");
        assert_eq!(invalidated.object_refs_invalidated, 1);
        assert_eq!(invalidated.derived_records_invalidated, 1);

        let idempotent = backend
            .invalidate_trace_submission_artifacts(
                tenant_id,
                submission_id,
                TraceDerivedStatus::Revoked,
            )
            .await
            .expect("repeat artifact invalidation");
        assert_eq!(idempotent.object_refs_invalidated, 0);
        assert_eq!(idempotent.derived_records_invalidated, 0);

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
