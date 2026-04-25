#[cfg(feature = "libsql")]
mod libsql_trace_corpus_store {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use chrono::Utc;
    use ironclaw::db::{Database, libsql::LibSqlBackend};
    use ironclaw::trace_corpus_storage::{
        TenantScopedTraceObjectRef, TraceAuditAction, TraceAuditEventWrite, TraceAuditSafeMetadata,
        TraceCorpusStatus, TraceCorpusStore, TraceCreditEventType, TraceCreditEventWrite,
        TraceCreditSettlementState, TraceDerivedRecordWrite, TraceDerivedStatus,
        TraceExportManifestWrite, TraceObjectArtifactKind, TraceObjectRefWrite,
        TraceSubmissionWrite, TraceTombstoneWrite, TraceVectorEntrySourceProjection,
        TraceVectorEntryStatus, TraceVectorEntryWrite, TraceWorkerKind,
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

        let object_refs = backend
            .list_trace_object_refs(tenant_id, submission_id)
            .await
            .expect("list object refs for tenant submission");
        assert_eq!(object_refs.len(), 1);
        assert_eq!(object_refs[0].tenant_id, tenant_id);
        assert_eq!(object_refs[0].submission_id, submission_id);
        assert_eq!(object_refs[0].object_ref_id, object_ref_id);
        assert_eq!(
            object_refs[0].artifact_kind,
            TraceObjectArtifactKind::SubmittedEnvelope
        );
        assert_eq!(object_refs[0].object_store, "s3://private-corpus");
        assert_eq!(object_refs[0].object_key, "tenant-alpha/submission.json");
        assert_eq!(object_refs[0].content_sha256, "sha256:object");
        assert_eq!(object_refs[0].encryption_key_ref, "kms:tenant-alpha");
        assert_eq!(object_refs[0].size_bytes, 4096);
        assert!(object_refs[0].compression.is_none());
        assert!(object_refs[0].created_by_job_id.is_none());
        assert!(object_refs[0].invalidated_at.is_none());
        assert!(object_refs[0].deleted_at.is_none());
        assert!(object_refs[0].created_at <= object_refs[0].updated_at);

        backend
            .upsert_trace_submission(sample_submission("tenant-beta", submission_id))
            .await
            .expect("insert same submission id for other tenant");
        backend
            .append_trace_object_ref(TraceObjectRefWrite {
                tenant_id: "tenant-beta".to_string(),
                object_ref_id: Uuid::new_v4(),
                submission_id,
                artifact_kind: TraceObjectArtifactKind::SubmittedEnvelope,
                object_store: "s3://private-corpus".to_string(),
                object_key: "tenant-beta/submission.json".to_string(),
                content_sha256: "sha256:other-tenant-object".to_string(),
                encryption_key_ref: "kms:tenant-beta".to_string(),
                size_bytes: 2048,
                compression: Some("zstd".to_string()),
                created_by_job_id: None,
            })
            .await
            .expect("append object ref for other tenant");

        tokio::time::sleep(Duration::from_millis(5)).await;

        let latest_object_ref_id = Uuid::new_v4();
        backend
            .append_trace_object_ref(TraceObjectRefWrite {
                tenant_id: tenant_id.to_string(),
                object_ref_id: latest_object_ref_id,
                submission_id,
                artifact_kind: TraceObjectArtifactKind::SubmittedEnvelope,
                object_store: "s3://private-corpus".to_string(),
                object_key: "tenant-alpha/submission-v2.json".to_string(),
                content_sha256: "sha256:object-v2".to_string(),
                encryption_key_ref: "kms:tenant-alpha".to_string(),
                size_bytes: 8192,
                compression: Some("zstd".to_string()),
                created_by_job_id: Some(Uuid::new_v4()),
            })
            .await
            .expect("append newer object ref");

        let object_refs = backend
            .list_trace_object_refs(tenant_id, submission_id)
            .await
            .expect("list object refs after newer append");
        assert_eq!(object_refs.len(), 2);
        assert!(object_refs.iter().all(|ref_| ref_.tenant_id == tenant_id));
        assert!(
            object_refs
                .iter()
                .all(|ref_| ref_.submission_id == submission_id)
        );
        assert_eq!(object_refs[0].object_ref_id, object_ref_id);
        assert_eq!(object_refs[1].object_ref_id, latest_object_ref_id);

        let other_tenant_object_refs = backend
            .list_trace_object_refs("tenant-beta", submission_id)
            .await
            .expect("list object refs for other tenant submission");
        assert_eq!(other_tenant_object_refs.len(), 1);
        assert_eq!(
            other_tenant_object_refs[0].object_key,
            "tenant-beta/submission.json"
        );

        let latest_active = backend
            .get_latest_active_trace_object_ref(
                tenant_id,
                submission_id,
                TraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .expect("get latest active object ref")
            .expect("latest active object ref exists");
        assert_eq!(latest_active.object_ref_id, latest_object_ref_id);
        assert_eq!(latest_active.object_key, "tenant-alpha/submission-v2.json");
        assert_eq!(latest_active.content_sha256, "sha256:object-v2");
        assert_eq!(latest_active.compression.as_deref(), Some("zstd"));
        assert!(latest_active.created_by_job_id.is_some());

        let other_tenant_latest_active = backend
            .get_latest_active_trace_object_ref(
                "tenant-beta",
                submission_id,
                TraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .expect("get latest active object ref for other tenant")
            .expect("other tenant latest active object ref exists");
        assert_eq!(
            other_tenant_latest_active.object_key,
            "tenant-beta/submission.json"
        );

        let missing_kind = backend
            .get_latest_active_trace_object_ref(
                tenant_id,
                submission_id,
                TraceObjectArtifactKind::ReviewSnapshot,
            )
            .await
            .expect("get latest active object ref for missing kind");
        assert!(missing_kind.is_none());

        let derived_id = Uuid::new_v4();
        backend
            .append_trace_derived_record(TraceDerivedRecordWrite {
                tenant_id: tenant_id.to_string(),
                derived_id,
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

        let vector_entry_id = Uuid::new_v4();
        backend
            .upsert_trace_vector_entry(TraceVectorEntryWrite {
                tenant_id: tenant_id.to_string(),
                submission_id,
                derived_id,
                vector_entry_id,
                vector_store: "trace-commons-main".to_string(),
                embedding_model: "text-embedding-3-small".to_string(),
                embedding_dimension: 1536,
                embedding_version: "embedding-v1".to_string(),
                source_projection: TraceVectorEntrySourceProjection::CanonicalSummary,
                source_hash: "sha256:canonical".to_string(),
                status: TraceVectorEntryStatus::Active,
                nearest_trace_ids: vec!["trace:near-1".to_string(), "trace:near-2".to_string()],
                cluster_id: Some("cluster:test".to_string()),
                duplicate_score: Some(0.1),
                novelty_score: Some(0.7),
                indexed_at: Some(Utc::now()),
                invalidated_at: None,
                deleted_at: None,
            })
            .await
            .expect("upsert vector entry");

        let vector_entries = backend
            .list_trace_vector_entries(tenant_id)
            .await
            .expect("list vector entries for tenant");
        assert_eq!(vector_entries.len(), 1);
        assert_eq!(vector_entries[0].tenant_id, tenant_id);
        assert_eq!(vector_entries[0].submission_id, submission_id);
        assert_eq!(vector_entries[0].derived_id, derived_id);
        assert_eq!(vector_entries[0].vector_entry_id, vector_entry_id);
        assert_eq!(vector_entries[0].vector_store, "trace-commons-main");
        assert_eq!(vector_entries[0].embedding_model, "text-embedding-3-small");
        assert_eq!(vector_entries[0].embedding_dimension, 1536);
        assert_eq!(vector_entries[0].embedding_version, "embedding-v1");
        assert_eq!(
            vector_entries[0].source_projection,
            TraceVectorEntrySourceProjection::CanonicalSummary
        );
        assert_eq!(vector_entries[0].source_hash, "sha256:canonical");
        assert_eq!(vector_entries[0].status, TraceVectorEntryStatus::Active);
        assert_eq!(
            vector_entries[0].nearest_trace_ids,
            vec!["trace:near-1", "trace:near-2"]
        );
        assert_eq!(
            vector_entries[0].cluster_id.as_deref(),
            Some("cluster:test")
        );
        assert_eq!(vector_entries[0].duplicate_score, Some(0.1));
        assert_eq!(vector_entries[0].novelty_score, Some(0.7));
        assert!(vector_entries[0].indexed_at.is_some());
        assert!(vector_entries[0].invalidated_at.is_none());
        assert!(vector_entries[0].deleted_at.is_none());
        assert!(vector_entries[0].created_at <= vector_entries[0].updated_at);

        let other_tenant_vector_entries = backend
            .list_trace_vector_entries("tenant-beta")
            .await
            .expect("list vector entries for other tenant");
        assert!(other_tenant_vector_entries.is_empty());

        let invalidated_vectors = backend
            .invalidate_trace_vector_entries_for_submission(tenant_id, submission_id)
            .await
            .expect("invalidate vector entries");
        assert_eq!(invalidated_vectors, 1);

        let vector_entries = backend
            .list_trace_vector_entries(tenant_id)
            .await
            .expect("list invalidated vector entries");
        assert_eq!(vector_entries.len(), 1);
        assert_eq!(
            vector_entries[0].status,
            TraceVectorEntryStatus::Invalidated
        );
        assert!(vector_entries[0].invalidated_at.is_some());

        let idempotent_vectors = backend
            .invalidate_trace_vector_entries_for_submission(tenant_id, submission_id)
            .await
            .expect("repeat vector invalidation");
        assert_eq!(idempotent_vectors, 0);

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

        let audit_events = backend
            .list_trace_audit_events(tenant_id)
            .await
            .expect("list audit events for tenant");
        assert_eq!(audit_events.len(), 1);
        assert_eq!(audit_events[0].submission_id, Some(submission_id));
        assert_eq!(audit_events[0].action, TraceAuditAction::Submit);
        assert_eq!(audit_events[0].actor_principal_ref, "principal:test-user");
        assert_eq!(audit_events[0].request_id.as_deref(), Some("request:test"));
        assert_eq!(
            audit_events[0].metadata,
            TraceAuditSafeMetadata::Submission {
                status: TraceCorpusStatus::Accepted,
                privacy_risk: "low".to_string(),
            }
        );

        let other_tenant_audit_events = backend
            .list_trace_audit_events("tenant-beta")
            .await
            .expect("list audit events for other tenant");
        assert!(other_tenant_audit_events.is_empty());

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

        let other_tenant_same_submission = backend
            .get_trace_submission("tenant-beta", submission_id)
            .await
            .expect("get same submission id for other tenant")
            .expect("other tenant submission should exist independently");
        assert_eq!(other_tenant_same_submission.tenant_id, "tenant-beta");
        assert_eq!(other_tenant_same_submission.submission_id, submission_id);

        let missing_tenant = backend
            .get_trace_submission("tenant-gamma", submission_id)
            .await
            .expect("tenant-isolated get");
        assert!(missing_tenant.is_none());

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
        assert_eq!(invalidated.object_refs_invalidated, 2);
        assert_eq!(invalidated.derived_records_invalidated, 1);

        let latest_after_invalidation = backend
            .get_latest_active_trace_object_ref(
                tenant_id,
                submission_id,
                TraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .expect("get latest active object ref after invalidation");
        assert!(latest_after_invalidation.is_none());

        let invalidated_object_refs = backend
            .list_trace_object_refs(tenant_id, submission_id)
            .await
            .expect("list object refs after invalidation");
        assert_eq!(invalidated_object_refs.len(), 2);
        assert!(
            invalidated_object_refs
                .iter()
                .all(|ref_| ref_.invalidated_at.is_some())
        );
        assert!(
            invalidated_object_refs
                .iter()
                .all(|ref_| ref_.deleted_at.is_none())
        );

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

    #[tokio::test]
    async fn libsql_store_preserves_export_manifest_tenant_scope_and_invalidation() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir.path().join("trace-export-manifests.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");

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

        let alpha_manifests = backend
            .list_trace_export_manifests("tenant-alpha")
            .await
            .expect("list alpha export manifests");
        assert_eq!(alpha_manifests.len(), 1);
        assert_eq!(alpha_manifests[0].export_manifest_id, alpha_export_id);
        assert_eq!(
            alpha_manifests[0].artifact_kind,
            TraceObjectArtifactKind::ExportArtifact
        );
        assert_eq!(
            alpha_manifests[0].purpose_code.as_deref(),
            Some("ranking_dataset")
        );
        assert_eq!(
            alpha_manifests[0].source_submission_ids,
            vec![submission_id]
        );
        assert_eq!(
            alpha_manifests[0].source_submission_ids_hash,
            "sha256:alpha-sources"
        );
        assert_eq!(alpha_manifests[0].item_count, 1);
        assert!(alpha_manifests[0].invalidated_at.is_none());

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
            .expect("list invalidated alpha export manifests");
        assert!(alpha_manifests[0].invalidated_at.is_some());
        assert!(alpha_manifests[0].deleted_at.is_none());

        let beta_manifests = backend
            .list_trace_export_manifests("tenant-beta")
            .await
            .expect("list beta export manifests");
        assert_eq!(beta_manifests.len(), 1);
        assert_eq!(beta_manifests[0].export_manifest_id, beta_export_id);
        assert!(beta_manifests[0].invalidated_at.is_none());
    }
}
