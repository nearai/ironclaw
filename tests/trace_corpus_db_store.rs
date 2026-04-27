#[cfg(feature = "libsql")]
mod libsql_trace_corpus_store {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use chrono::{DateTime, Utc};
    use ironclaw::db::{Database, libsql::LibSqlBackend};
    use ironclaw::trace_corpus_storage::{
        TenantScopedTraceObjectRef, TraceAuditAction, TraceAuditEventWrite, TraceAuditSafeMetadata,
        TraceCorpusStatus, TraceCorpusStore, TraceCreditEventType, TraceCreditEventWrite,
        TraceCreditSettlementState, TraceDerivedRecordWrite, TraceDerivedStatus,
        TraceExportAccessGrantStatus, TraceExportAccessGrantWrite, TraceExportJobStatus,
        TraceExportJobStatusUpdate, TraceExportJobWrite, TraceExportManifestItemInvalidationReason,
        TraceExportManifestItemWrite, TraceExportManifestMirrorWrite, TraceExportManifestWrite,
        TraceObjectArtifactKind, TraceObjectRefWrite, TraceRetentionJobItemAction,
        TraceRetentionJobItemStatus, TraceRetentionJobItemWrite, TraceRetentionJobStatus,
        TraceRetentionJobWrite, TraceReviewLeaseAuditAction, TraceRevocationPropagationAction,
        TraceRevocationPropagationItemStatus, TraceRevocationPropagationItemStatusUpdate,
        TraceRevocationPropagationItemWrite, TraceRevocationPropagationTarget,
        TraceSubmissionWrite, TraceTenantAccessGrantRole, TraceTenantAccessGrantStatus,
        TraceTenantAccessGrantWrite, TraceTenantPolicyWrite, TraceTombstoneWrite,
        TraceVectorEntrySourceProjection, TraceVectorEntryStatus, TraceVectorEntryWrite,
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
            actor_principal_ref: "principal:test-user".to_string(),
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
            actor_principal_ref: "principal:test-user".to_string(),
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

        let policy = backend
            .upsert_trace_tenant_policy(TraceTenantPolicyWrite {
                tenant_id: tenant_id.to_string(),
                policy_version: "tenant-policy-v1".to_string(),
                allowed_consent_scopes: vec!["debugging_evaluation".to_string()],
                allowed_uses: vec!["debugging".to_string(), "evaluation".to_string()],
                updated_by_principal_ref: "admin:test".to_string(),
            })
            .await
            .expect("upsert tenant policy");
        assert_eq!(policy.tenant_id, tenant_id);
        assert_eq!(policy.policy_version, "tenant-policy-v1");
        assert_eq!(policy.allowed_consent_scopes, vec!["debugging_evaluation"]);
        assert_eq!(policy.allowed_uses, vec!["debugging", "evaluation"]);
        assert_eq!(policy.updated_by_principal_ref, "admin:test");

        let read_policy = backend
            .get_trace_tenant_policy(tenant_id)
            .await
            .expect("read tenant policy")
            .expect("tenant policy exists");
        assert_eq!(read_policy, policy);
        assert!(
            backend
                .get_trace_tenant_policy("tenant-beta")
                .await
                .expect("read other tenant policy")
                .is_none()
        );

        let updated_policy = backend
            .upsert_trace_tenant_policy(TraceTenantPolicyWrite {
                tenant_id: tenant_id.to_string(),
                policy_version: "tenant-policy-v2".to_string(),
                allowed_consent_scopes: vec![
                    "debugging_evaluation".to_string(),
                    "benchmark_only".to_string(),
                ],
                allowed_uses: vec!["debugging".to_string()],
                updated_by_principal_ref: "admin:second".to_string(),
            })
            .await
            .expect("update tenant policy");
        assert_eq!(updated_policy.policy_version, "tenant-policy-v2");
        assert_eq!(
            updated_policy.allowed_consent_scopes,
            vec!["debugging_evaluation", "benchmark_only"]
        );
        assert_eq!(updated_policy.allowed_uses, vec!["debugging"]);
        assert_eq!(updated_policy.updated_by_principal_ref, "admin:second");

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
                previous_event_hash: Some("sha256:genesis".to_string()),
                event_hash: Some("sha256:test-audit-event".to_string()),
                canonical_event_json: Some("{\"kind\":\"submitted\"}".to_string()),
                metadata: TraceAuditSafeMetadata::Submission {
                    status: TraceCorpusStatus::Accepted,
                    privacy_risk: "low".to_string(),
                },
            })
            .await
            .expect("append audit event");
        let lease_expires_at = Utc::now() + chrono::Duration::minutes(15);
        backend
            .append_trace_audit_event(TraceAuditEventWrite {
                tenant_id: tenant_id.to_string(),
                audit_event_id: Uuid::new_v4(),
                submission_id: Some(submission_id),
                actor_principal_ref: "principal:reviewer".to_string(),
                actor_role: "reviewer".to_string(),
                action: TraceAuditAction::Review,
                reason: Some("action=claim".to_string()),
                request_id: None,
                object_ref_id: None,
                export_manifest_id: None,
                decision_inputs_hash: None,
                previous_event_hash: Some("sha256:test-audit-event".to_string()),
                event_hash: Some("sha256:test-review-lease-event".to_string()),
                canonical_event_json: Some("{\"kind\":\"review_lease\"}".to_string()),
                metadata: TraceAuditSafeMetadata::ReviewLease {
                    action: TraceReviewLeaseAuditAction::Claim,
                    lease_expires_at: Some(lease_expires_at),
                    review_due_at: None,
                },
            })
            .await
            .expect("append review lease audit event");

        let audit_events = backend
            .list_trace_audit_events(tenant_id)
            .await
            .expect("list audit events for tenant");
        assert_eq!(audit_events.len(), 2);
        assert_eq!(audit_events[0].submission_id, Some(submission_id));
        assert_eq!(audit_events[0].action, TraceAuditAction::Submit);
        assert_eq!(audit_events[0].actor_principal_ref, "principal:test-user");
        assert_eq!(audit_events[0].request_id.as_deref(), Some("request:test"));
        assert_eq!(
            audit_events[0].previous_event_hash.as_deref(),
            Some("sha256:genesis")
        );
        assert_eq!(
            audit_events[0].event_hash.as_deref(),
            Some("sha256:test-audit-event")
        );
        assert_eq!(
            audit_events[0].canonical_event_json.as_deref(),
            Some("{\"kind\":\"submitted\"}")
        );
        assert_eq!(
            audit_events[0].metadata,
            TraceAuditSafeMetadata::Submission {
                status: TraceCorpusStatus::Accepted,
                privacy_risk: "low".to_string(),
            }
        );
        assert_eq!(audit_events[1].submission_id, Some(submission_id));
        assert_eq!(audit_events[1].action, TraceAuditAction::Review);
        assert_eq!(
            audit_events[1].previous_event_hash.as_deref(),
            Some("sha256:test-audit-event")
        );
        assert_eq!(
            audit_events[1].event_hash.as_deref(),
            Some("sha256:test-review-lease-event")
        );
        assert_eq!(
            audit_events[1].metadata,
            TraceAuditSafeMetadata::ReviewLease {
                action: TraceReviewLeaseAuditAction::Claim,
                lease_expires_at: Some(lease_expires_at),
                review_due_at: None,
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
        backend
            .append_trace_credit_event(TraceCreditEventWrite {
                tenant_id: tenant_id.to_string(),
                credit_event_id: Uuid::new_v4(),
                submission_id,
                trace_id: inserted.trace_id,
                credit_account_ref: "credit:test".to_string(),
                event_type: TraceCreditEventType::RankingUtility,
                points_delta: "0.75".to_string(),
                reason: "Ranker pair utility.".to_string(),
                external_ref: Some("ranker_training_pairs_export:test".to_string()),
                actor_principal_ref: "principal:reviewer".to_string(),
                actor_role: "reviewer".to_string(),
                settlement_state: TraceCreditSettlementState::Final,
            })
            .await
            .expect("append ranking utility credit event");

        let credit_events = backend
            .list_trace_credit_events(tenant_id)
            .await
            .expect("list credit events for tenant");
        assert_eq!(credit_events.len(), 2);
        let accepted_credit = credit_events
            .iter()
            .find(|event| event.event_type == TraceCreditEventType::Accepted)
            .expect("accepted credit event round-trips");
        assert_eq!(accepted_credit.submission_id, submission_id);
        assert_eq!(accepted_credit.trace_id, inserted.trace_id);
        assert_eq!(accepted_credit.credit_account_ref, "credit:test");
        assert_eq!(accepted_credit.points_delta, "1.0");
        assert_eq!(
            accepted_credit.settlement_state,
            TraceCreditSettlementState::Pending
        );
        assert_eq!(accepted_credit.actor_role, "contributor");
        let ranking_credit = credit_events
            .iter()
            .find(|event| event.event_type == TraceCreditEventType::RankingUtility)
            .expect("ranking utility credit event round-trips");
        assert_eq!(ranking_credit.submission_id, submission_id);
        assert_eq!(ranking_credit.points_delta, "0.75");
        assert_eq!(
            ranking_credit.external_ref.as_deref(),
            Some("ranker_training_pairs_export:test")
        );
        assert_eq!(
            ranking_credit.settlement_state,
            TraceCreditSettlementState::Final
        );
        assert_eq!(ranking_credit.actor_role, "reviewer");

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
        let revoked_submission = backend
            .get_trace_submission(tenant_id, submission_id)
            .await
            .expect("get revoked submission")
            .expect("revoked submission exists");
        assert_eq!(revoked_submission.status, TraceCorpusStatus::Revoked);
        assert_eq!(revoked_submission.credit_points_pending, Some(0.0));
        assert_eq!(revoked_submission.credit_points_final, Some(0.0));

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
        let deleted_count = backend
            .mark_trace_object_ref_deleted(
                tenant_id,
                submission_id,
                "s3://private-corpus",
                "tenant-alpha/submission.json",
            )
            .await
            .expect("mark exact object ref deleted");
        assert_eq!(deleted_count, 1);
        let object_refs_after_delete = backend
            .list_trace_object_refs(tenant_id, submission_id)
            .await
            .expect("list object refs after exact delete");
        let deleted_ref = object_refs_after_delete
            .iter()
            .find(|ref_| ref_.object_ref_id == object_ref_id)
            .expect("deleted object ref remains listed");
        assert!(deleted_ref.deleted_at.is_some());
        let untouched_ref = object_refs_after_delete
            .iter()
            .find(|ref_| ref_.object_ref_id == latest_object_ref_id)
            .expect("untouched object ref remains listed");
        assert!(untouched_ref.deleted_at.is_none());
        let idempotent_delete = backend
            .mark_trace_object_ref_deleted(
                tenant_id,
                submission_id,
                "s3://private-corpus",
                "tenant-alpha/submission.json",
            )
            .await
            .expect("repeat exact object ref delete");
        assert_eq!(idempotent_delete, 0);

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

        let tombstone_id = Uuid::new_v4();
        let effective_at = DateTime::parse_from_rfc3339("2026-04-25T12:00:00Z")
            .expect("parse effective timestamp")
            .with_timezone(&Utc);
        let retain_until = DateTime::parse_from_rfc3339("2026-05-25T12:00:00Z")
            .expect("parse retain-until timestamp")
            .with_timezone(&Utc);
        backend
            .write_trace_tombstone(TraceTombstoneWrite {
                tombstone_id,
                tenant_id: tenant_id.to_string(),
                submission_id,
                trace_id: Some(inserted.trace_id),
                redaction_hash: Some("sha256:redaction".to_string()),
                canonical_summary_hash: Some("sha256:canonical".to_string()),
                reason: "user requested revocation".to_string(),
                effective_at,
                retain_until: Some(retain_until),
                created_by_principal_ref: "principal:test-user".to_string(),
            })
            .await
            .expect("append tombstone");

        let later_effective_at = DateTime::parse_from_rfc3339("2026-04-26T12:00:00Z")
            .expect("parse later effective timestamp")
            .with_timezone(&Utc);
        backend
            .write_trace_tombstone(TraceTombstoneWrite {
                tombstone_id: Uuid::new_v4(),
                tenant_id: tenant_id.to_string(),
                submission_id,
                trace_id: Some(inserted.trace_id),
                redaction_hash: Some("sha256:later-redaction".to_string()),
                canonical_summary_hash: Some("sha256:later-canonical".to_string()),
                reason: "later duplicate revocation".to_string(),
                effective_at: later_effective_at,
                retain_until: None,
                created_by_principal_ref: "principal:later-user".to_string(),
            })
            .await
            .expect("repeat tombstone write is idempotent");

        backend
            .write_trace_tombstone(TraceTombstoneWrite {
                tombstone_id: Uuid::new_v4(),
                tenant_id: "tenant-beta".to_string(),
                submission_id,
                trace_id: None,
                redaction_hash: Some("sha256:other-tenant-redaction".to_string()),
                canonical_summary_hash: None,
                reason: "other tenant revocation".to_string(),
                effective_at,
                retain_until: None,
                created_by_principal_ref: "principal:other-tenant-user".to_string(),
            })
            .await
            .expect("append other-tenant tombstone");

        let tombstones = backend
            .list_trace_tombstones(tenant_id)
            .await
            .expect("list tombstones for tenant");
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].tenant_id, tenant_id);
        assert_eq!(tombstones[0].tombstone_id, tombstone_id);
        assert_eq!(tombstones[0].submission_id, submission_id);
        assert_eq!(tombstones[0].trace_id, Some(inserted.trace_id));
        assert_eq!(
            tombstones[0].redaction_hash.as_deref(),
            Some("sha256:redaction")
        );
        assert_eq!(
            tombstones[0].canonical_summary_hash.as_deref(),
            Some("sha256:canonical")
        );
        assert_eq!(tombstones[0].reason, "user requested revocation");
        assert_eq!(tombstones[0].effective_at, effective_at);
        assert_eq!(tombstones[0].retain_until, Some(retain_until));
        assert_eq!(
            tombstones[0].created_by_principal_ref,
            "principal:test-user"
        );
        assert!(tombstones[0].created_at <= Utc::now());

        let other_tenant_tombstones = backend
            .list_trace_tombstones("tenant-beta")
            .await
            .expect("list tombstones for other tenant");
        assert_eq!(other_tenant_tombstones.len(), 1);
        assert_eq!(other_tenant_tombstones[0].tenant_id, "tenant-beta");
        assert_eq!(other_tenant_tombstones[0].reason, "other tenant revocation");

        let missing_tenant_tombstones = backend
            .list_trace_tombstones("tenant-gamma")
            .await
            .expect("list tombstones for missing tenant");
        assert!(missing_tenant_tombstones.is_empty());

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

    #[tokio::test]
    async fn libsql_store_preserves_export_manifest_item_scope_and_invalidation() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir.path().join("trace-export-manifest-items.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");

        let submission_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        let mut alpha_submission = sample_submission("tenant-alpha", submission_id);
        alpha_submission.trace_id = trace_id;
        backend
            .upsert_trace_submission(alpha_submission)
            .await
            .expect("insert alpha submission");
        let mut beta_submission = sample_submission("tenant-beta", submission_id);
        beta_submission.trace_id = trace_id;
        backend
            .upsert_trace_submission(beta_submission)
            .await
            .expect("insert beta submission");

        let alpha_export_id = Uuid::new_v4();
        let beta_export_id = Uuid::new_v4();
        backend
            .upsert_trace_export_manifest(TraceExportManifestWrite {
                tenant_id: "tenant-alpha".to_string(),
                export_manifest_id: alpha_export_id,
                artifact_kind: TraceObjectArtifactKind::ExportArtifact,
                purpose_code: Some("replay_dataset".to_string()),
                audit_event_id: Some(Uuid::new_v4()),
                source_submission_ids: vec![submission_id],
                source_submission_ids_hash: "sha256:alpha-sources".to_string(),
                item_count: 1,
                generated_at: Utc::now(),
            })
            .await
            .expect("insert alpha manifest");
        backend
            .upsert_trace_export_manifest(TraceExportManifestWrite {
                tenant_id: "tenant-beta".to_string(),
                export_manifest_id: beta_export_id,
                artifact_kind: TraceObjectArtifactKind::ExportArtifact,
                purpose_code: Some("replay_dataset".to_string()),
                audit_event_id: Some(Uuid::new_v4()),
                source_submission_ids: vec![submission_id],
                source_submission_ids_hash: "sha256:beta-sources".to_string(),
                item_count: 1,
                generated_at: Utc::now(),
            })
            .await
            .expect("insert beta manifest");

        let alpha_object_ref_id = Uuid::new_v4();
        let alpha_derived_id = Uuid::new_v4();
        let alpha_vector_entry_id = Uuid::new_v4();
        backend
            .append_trace_object_ref(TraceObjectRefWrite {
                tenant_id: "tenant-alpha".to_string(),
                object_ref_id: alpha_object_ref_id,
                submission_id,
                artifact_kind: TraceObjectArtifactKind::WorkerIntermediate,
                object_store: "s3://private-corpus".to_string(),
                object_key: "tenant-alpha/worker/summary.json".to_string(),
                content_sha256: "sha256:alpha-object".to_string(),
                encryption_key_ref: "kms:tenant-alpha".to_string(),
                size_bytes: 128,
                compression: None,
                created_by_job_id: None,
            })
            .await
            .expect("insert alpha object ref");
        backend
            .append_trace_derived_record(TraceDerivedRecordWrite {
                tenant_id: "tenant-alpha".to_string(),
                derived_id: alpha_derived_id,
                submission_id,
                trace_id,
                status: TraceDerivedStatus::Current,
                worker_kind: TraceWorkerKind::Summary,
                worker_version: "summary-worker-v1".to_string(),
                input_object_ref: Some(TenantScopedTraceObjectRef {
                    tenant_id: "tenant-alpha".to_string(),
                    submission_id,
                    object_ref_id: alpha_object_ref_id,
                }),
                input_hash: "sha256:alpha-object".to_string(),
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
            .expect("insert alpha derived record");
        backend
            .upsert_trace_vector_entry(TraceVectorEntryWrite {
                tenant_id: "tenant-alpha".to_string(),
                submission_id,
                derived_id: alpha_derived_id,
                vector_entry_id: alpha_vector_entry_id,
                vector_store: "trace-commons-main".to_string(),
                embedding_model: "text-embedding-3-small".to_string(),
                embedding_dimension: 1536,
                embedding_version: "embedding-v1".to_string(),
                source_projection: TraceVectorEntrySourceProjection::CanonicalSummary,
                source_hash: "sha256:alpha-summary".to_string(),
                status: TraceVectorEntryStatus::Active,
                nearest_trace_ids: Vec::new(),
                cluster_id: Some("cluster:alpha".to_string()),
                duplicate_score: Some(0.1),
                novelty_score: Some(0.4),
                indexed_at: Some(Utc::now()),
                invalidated_at: None,
                deleted_at: None,
            })
            .await
            .expect("insert alpha vector entry");

        backend
            .upsert_trace_export_manifest_item(TraceExportManifestItemWrite {
                tenant_id: "tenant-alpha".to_string(),
                export_manifest_id: alpha_export_id,
                submission_id,
                trace_id,
                derived_id: Some(alpha_derived_id),
                object_ref_id: Some(alpha_object_ref_id),
                vector_entry_id: Some(alpha_vector_entry_id),
                source_status_at_export: TraceCorpusStatus::Accepted,
                source_hash_at_export: "sha256:alpha-source".to_string(),
            })
            .await
            .expect("insert alpha manifest item");
        backend
            .upsert_trace_export_manifest_item(TraceExportManifestItemWrite {
                tenant_id: "tenant-beta".to_string(),
                export_manifest_id: beta_export_id,
                submission_id,
                trace_id,
                derived_id: None,
                object_ref_id: None,
                vector_entry_id: None,
                source_status_at_export: TraceCorpusStatus::Accepted,
                source_hash_at_export: "sha256:beta-source".to_string(),
            })
            .await
            .expect("insert beta manifest item");

        let alpha_items = backend
            .list_trace_export_manifest_items("tenant-alpha", alpha_export_id)
            .await
            .expect("list alpha manifest items");
        assert_eq!(alpha_items.len(), 1);
        assert_eq!(alpha_items[0].tenant_id, "tenant-alpha");
        assert_eq!(alpha_items[0].export_manifest_id, alpha_export_id);
        assert_eq!(alpha_items[0].submission_id, submission_id);
        assert_eq!(alpha_items[0].trace_id, trace_id);
        assert_eq!(
            alpha_items[0].source_status_at_export,
            TraceCorpusStatus::Accepted
        );
        assert_eq!(alpha_items[0].source_hash_at_export, "sha256:alpha-source");
        assert!(alpha_items[0].derived_id.is_some());
        assert!(alpha_items[0].object_ref_id.is_some());
        assert!(alpha_items[0].vector_entry_id.is_some());
        assert!(alpha_items[0].source_invalidated_at.is_none());
        assert!(alpha_items[0].source_invalidation_reason.is_none());

        let invalidated = backend
            .invalidate_trace_export_manifest_items_for_submission(
                "tenant-alpha",
                submission_id,
                TraceExportManifestItemInvalidationReason::Revoked,
            )
            .await
            .expect("invalidate alpha manifest item");
        assert_eq!(invalidated, 1);
        let idempotent = backend
            .invalidate_trace_export_manifest_items_for_submission(
                "tenant-alpha",
                submission_id,
                TraceExportManifestItemInvalidationReason::Revoked,
            )
            .await
            .expect("repeat alpha manifest item invalidation");
        assert_eq!(idempotent, 0);

        let alpha_items = backend
            .list_trace_export_manifest_items("tenant-alpha", alpha_export_id)
            .await
            .expect("list invalidated alpha manifest items");
        assert!(alpha_items[0].source_invalidated_at.is_some());
        assert_eq!(
            alpha_items[0].source_invalidation_reason,
            Some(TraceExportManifestItemInvalidationReason::Revoked)
        );

        let beta_items = backend
            .list_trace_export_manifest_items("tenant-beta", beta_export_id)
            .await
            .expect("list beta manifest items");
        assert_eq!(beta_items.len(), 1);
        assert!(beta_items[0].source_invalidated_at.is_none());
    }

    #[tokio::test]
    async fn libsql_store_rolls_back_export_manifest_mirror_when_item_ref_is_invalid() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir.path().join("trace-export-manifest-mirror.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");

        let tenant_id = "tenant-alpha";
        let submission_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        let mut submission = sample_submission(tenant_id, submission_id);
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
                tenant_id: tenant_id.to_string(),
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
                    tenant_id: tenant_id.to_string(),
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
                    tenant_id: tenant_id.to_string(),
                    object_ref_id,
                    submission_id,
                    artifact_kind: TraceObjectArtifactKind::BenchmarkArtifact,
                    object_store: "trace_commons_file_store".to_string(),
                    object_key: "tenants/tenant-alpha/benchmarks/export/artifact.json".to_string(),
                    content_sha256: "sha256:artifact".to_string(),
                    encryption_key_ref: "tenant:tenant-alpha".to_string(),
                    size_bytes: 128,
                    compression: None,
                    created_by_job_id: Some(export_id),
                }],
                items: vec![
                    TraceExportManifestItemWrite {
                        tenant_id: tenant_id.to_string(),
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
                        tenant_id: tenant_id.to_string(),
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
            matches!(error, ironclaw::error::DatabaseError::NotFound { .. }),
            "unexpected mirror error: {error}"
        );

        let manifests = backend
            .list_trace_export_manifests(tenant_id)
            .await
            .expect("list manifests after failed mirror");
        assert!(manifests.is_empty());
        let items = backend
            .list_trace_export_manifest_items(tenant_id, export_id)
            .await
            .expect("list manifest items after failed mirror");
        assert!(items.is_empty());
        let object_refs = backend
            .list_trace_object_refs(tenant_id, submission_id)
            .await
            .expect("list object refs after failed mirror");
        assert!(
            object_refs.is_empty(),
            "failed mirror must roll back staged export object refs"
        );
    }

    #[tokio::test]
    async fn libsql_store_rejects_export_manifest_item_cross_tenant_refs() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir
            .path()
            .join("trace-export-manifest-cross-tenant-refs.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");

        let submission_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        let mut alpha_submission = sample_submission("tenant-alpha", submission_id);
        alpha_submission.trace_id = trace_id;
        backend
            .upsert_trace_submission(alpha_submission)
            .await
            .expect("insert alpha submission");
        let mut beta_submission = sample_submission("tenant-beta", submission_id);
        beta_submission.trace_id = trace_id;
        backend
            .upsert_trace_submission(beta_submission)
            .await
            .expect("insert beta submission");

        let beta_object_ref_id = Uuid::new_v4();
        let beta_derived_id = Uuid::new_v4();
        let beta_vector_entry_id = Uuid::new_v4();
        backend
            .append_trace_object_ref(TraceObjectRefWrite {
                tenant_id: "tenant-beta".to_string(),
                object_ref_id: beta_object_ref_id,
                submission_id,
                artifact_kind: TraceObjectArtifactKind::WorkerIntermediate,
                object_store: "s3://private-corpus".to_string(),
                object_key: "tenant-beta/worker/summary.json".to_string(),
                content_sha256: "sha256:beta-object".to_string(),
                encryption_key_ref: "kms:tenant-beta".to_string(),
                size_bytes: 128,
                compression: None,
                created_by_job_id: None,
            })
            .await
            .expect("insert beta object ref");
        backend
            .append_trace_derived_record(TraceDerivedRecordWrite {
                tenant_id: "tenant-beta".to_string(),
                derived_id: beta_derived_id,
                submission_id,
                trace_id,
                status: TraceDerivedStatus::Current,
                worker_kind: TraceWorkerKind::Summary,
                worker_version: "summary-worker-v1".to_string(),
                input_object_ref: Some(TenantScopedTraceObjectRef {
                    tenant_id: "tenant-beta".to_string(),
                    submission_id,
                    object_ref_id: beta_object_ref_id,
                }),
                input_hash: "sha256:beta-object".to_string(),
                output_object_ref: None,
                canonical_summary: Some("Tenant beta summary.".to_string()),
                canonical_summary_hash: Some("sha256:beta-summary".to_string()),
                summary_model: "summary-model-v1".to_string(),
                task_success: Some("success".to_string()),
                privacy_risk: Some("low".to_string()),
                event_count: Some(2),
                tool_sequence: vec!["memory_search".to_string()],
                tool_categories: vec!["memory".to_string()],
                coverage_tags: vec!["tool:memory_search".to_string()],
                duplicate_score: Some(0.1),
                novelty_score: Some(0.4),
                cluster_id: Some("cluster:beta".to_string()),
            })
            .await
            .expect("insert beta derived record");
        backend
            .upsert_trace_vector_entry(TraceVectorEntryWrite {
                tenant_id: "tenant-beta".to_string(),
                submission_id,
                derived_id: beta_derived_id,
                vector_entry_id: beta_vector_entry_id,
                vector_store: "trace-commons-main".to_string(),
                embedding_model: "text-embedding-3-small".to_string(),
                embedding_dimension: 1536,
                embedding_version: "embedding-v1".to_string(),
                source_projection: TraceVectorEntrySourceProjection::CanonicalSummary,
                source_hash: "sha256:beta-summary".to_string(),
                status: TraceVectorEntryStatus::Active,
                nearest_trace_ids: Vec::new(),
                cluster_id: Some("cluster:beta".to_string()),
                duplicate_score: Some(0.1),
                novelty_score: Some(0.4),
                indexed_at: Some(Utc::now()),
                invalidated_at: None,
                deleted_at: None,
            })
            .await
            .expect("insert beta vector entry");

        let alpha_export_id = Uuid::new_v4();
        backend
            .upsert_trace_export_manifest(TraceExportManifestWrite {
                tenant_id: "tenant-alpha".to_string(),
                export_manifest_id: alpha_export_id,
                artifact_kind: TraceObjectArtifactKind::ExportArtifact,
                purpose_code: Some("replay_dataset".to_string()),
                audit_event_id: Some(Uuid::new_v4()),
                source_submission_ids: vec![submission_id],
                source_submission_ids_hash: "sha256:alpha-sources".to_string(),
                item_count: 1,
                generated_at: Utc::now(),
            })
            .await
            .expect("insert alpha manifest");

        let err = backend
            .upsert_trace_export_manifest_item(TraceExportManifestItemWrite {
                tenant_id: "tenant-alpha".to_string(),
                export_manifest_id: alpha_export_id,
                submission_id,
                trace_id,
                derived_id: Some(beta_derived_id),
                object_ref_id: Some(beta_object_ref_id),
                vector_entry_id: Some(beta_vector_entry_id),
                source_status_at_export: TraceCorpusStatus::Accepted,
                source_hash_at_export: "sha256:alpha-source".to_string(),
            })
            .await
            .expect_err("cross-tenant export refs must be rejected");

        assert!(
            err.to_string().contains("does not belong to tenant"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn libsql_store_rejects_derived_record_mismatched_tenant_object_ref() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir
            .path()
            .join("trace-derived-cross-tenant-object-ref.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");

        let submission_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        let mut alpha_submission = sample_submission("tenant-alpha", submission_id);
        alpha_submission.trace_id = trace_id;
        backend
            .upsert_trace_submission(alpha_submission)
            .await
            .expect("insert alpha submission");
        let mut beta_submission = sample_submission("tenant-beta", submission_id);
        beta_submission.trace_id = trace_id;
        backend
            .upsert_trace_submission(beta_submission)
            .await
            .expect("insert beta submission");

        let beta_object_ref_id = Uuid::new_v4();
        backend
            .append_trace_object_ref(TraceObjectRefWrite {
                tenant_id: "tenant-beta".to_string(),
                object_ref_id: beta_object_ref_id,
                submission_id,
                artifact_kind: TraceObjectArtifactKind::WorkerIntermediate,
                object_store: "s3://private-corpus".to_string(),
                object_key: "tenant-beta/worker/summary.json".to_string(),
                content_sha256: "sha256:beta-object".to_string(),
                encryption_key_ref: "kms:tenant-beta".to_string(),
                size_bytes: 128,
                compression: None,
                created_by_job_id: None,
            })
            .await
            .expect("insert beta object ref");

        let err = backend
            .append_trace_derived_record(TraceDerivedRecordWrite {
                tenant_id: "tenant-alpha".to_string(),
                derived_id: Uuid::new_v4(),
                submission_id,
                trace_id,
                status: TraceDerivedStatus::Current,
                worker_kind: TraceWorkerKind::Summary,
                worker_version: "summary-worker-v1".to_string(),
                input_object_ref: Some(TenantScopedTraceObjectRef {
                    tenant_id: "tenant-beta".to_string(),
                    submission_id,
                    object_ref_id: beta_object_ref_id,
                }),
                input_hash: "sha256:beta-object".to_string(),
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
            .expect_err("derived records must reject cross-tenant object refs");

        assert!(
            err.to_string().contains("does not belong to tenant"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn libsql_store_rejects_vector_entry_mismatched_submission_derived_id() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir
            .path()
            .join("trace-vector-mismatched-derived-id.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");

        let tenant_id = "tenant-alpha";
        let submission_a_id = Uuid::new_v4();
        let trace_a_id = Uuid::new_v4();
        let mut submission_a = sample_submission(tenant_id, submission_a_id);
        submission_a.trace_id = trace_a_id;
        backend
            .upsert_trace_submission(submission_a)
            .await
            .expect("insert submission A");

        let submission_b_id = Uuid::new_v4();
        let trace_b_id = Uuid::new_v4();
        let mut submission_b = sample_submission(tenant_id, submission_b_id);
        submission_b.trace_id = trace_b_id;
        backend
            .upsert_trace_submission(submission_b)
            .await
            .expect("insert submission B");

        let object_ref_b_id = Uuid::new_v4();
        let derived_b_id = Uuid::new_v4();
        backend
            .append_trace_object_ref(TraceObjectRefWrite {
                tenant_id: tenant_id.to_string(),
                object_ref_id: object_ref_b_id,
                submission_id: submission_b_id,
                artifact_kind: TraceObjectArtifactKind::WorkerIntermediate,
                object_store: "s3://private-corpus".to_string(),
                object_key: "tenant-alpha/submission-b/summary.json".to_string(),
                content_sha256: "sha256:submission-b-object".to_string(),
                encryption_key_ref: "kms:tenant-alpha".to_string(),
                size_bytes: 128,
                compression: None,
                created_by_job_id: None,
            })
            .await
            .expect("insert submission B object ref");
        backend
            .append_trace_derived_record(TraceDerivedRecordWrite {
                tenant_id: tenant_id.to_string(),
                derived_id: derived_b_id,
                submission_id: submission_b_id,
                trace_id: trace_b_id,
                status: TraceDerivedStatus::Current,
                worker_kind: TraceWorkerKind::Summary,
                worker_version: "summary-worker-v1".to_string(),
                input_object_ref: Some(TenantScopedTraceObjectRef {
                    tenant_id: tenant_id.to_string(),
                    submission_id: submission_b_id,
                    object_ref_id: object_ref_b_id,
                }),
                input_hash: "sha256:submission-b-object".to_string(),
                output_object_ref: None,
                canonical_summary: Some("Submission B summary.".to_string()),
                canonical_summary_hash: Some("sha256:submission-b-summary".to_string()),
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
            .expect("insert submission B derived record");

        let err = backend
            .upsert_trace_vector_entry(TraceVectorEntryWrite {
                tenant_id: tenant_id.to_string(),
                submission_id: submission_a_id,
                derived_id: derived_b_id,
                vector_entry_id: Uuid::new_v4(),
                vector_store: "trace-commons-main".to_string(),
                embedding_model: "text-embedding-3-small".to_string(),
                embedding_dimension: 1536,
                embedding_version: "embedding-v1".to_string(),
                source_projection: TraceVectorEntrySourceProjection::CanonicalSummary,
                source_hash: "sha256:submission-a-summary".to_string(),
                status: TraceVectorEntryStatus::Active,
                nearest_trace_ids: Vec::new(),
                cluster_id: Some("cluster:alpha".to_string()),
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
    }

    #[tokio::test]
    async fn libsql_store_invalidates_exact_vector_entry_with_tenant_submission_scope() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir.path().join("trace-exact-vector-invalidate.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");

        let submission_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        let target_derived_id = Uuid::new_v4();
        let sibling_derived_id = Uuid::new_v4();
        let target_vector_entry_id = Uuid::new_v4();
        let sibling_vector_entry_id = Uuid::new_v4();

        for tenant_id in ["tenant-alpha", "tenant-beta"] {
            let mut submission = sample_submission(tenant_id, submission_id);
            submission.trace_id = trace_id;
            backend
                .upsert_trace_submission(submission)
                .await
                .expect("insert scoped submission");
            for (derived_id, summary_hash) in [
                (target_derived_id, "sha256:target-summary"),
                (sibling_derived_id, "sha256:sibling-summary"),
            ] {
                backend
                    .append_trace_derived_record(TraceDerivedRecordWrite {
                        tenant_id: tenant_id.to_string(),
                        derived_id,
                        submission_id,
                        trace_id,
                        status: TraceDerivedStatus::Current,
                        worker_kind: TraceWorkerKind::DuplicatePrecheck,
                        worker_version: "duplicate-precheck-v1".to_string(),
                        input_object_ref: None,
                        input_hash: summary_hash.to_string(),
                        output_object_ref: None,
                        canonical_summary: Some(format!("{tenant_id} {summary_hash}")),
                        canonical_summary_hash: Some(summary_hash.to_string()),
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
                    .expect("insert scoped derived record");
            }
            for (derived_id, vector_entry_id, source_hash) in [
                (
                    target_derived_id,
                    target_vector_entry_id,
                    "sha256:target-summary",
                ),
                (
                    sibling_derived_id,
                    sibling_vector_entry_id,
                    "sha256:sibling-summary",
                ),
            ] {
                backend
                    .upsert_trace_vector_entry(TraceVectorEntryWrite {
                        tenant_id: tenant_id.to_string(),
                        submission_id,
                        derived_id,
                        vector_entry_id,
                        vector_store: "trace-commons-main".to_string(),
                        embedding_model: "redacted-summary-feature-hash-v1".to_string(),
                        embedding_dimension: 64,
                        embedding_version: "embedding-v1".to_string(),
                        source_projection: TraceVectorEntrySourceProjection::CanonicalSummary,
                        source_hash: source_hash.to_string(),
                        status: TraceVectorEntryStatus::Active,
                        nearest_trace_ids: Vec::new(),
                        cluster_id: Some("cluster:alpha".to_string()),
                        duplicate_score: Some(0.1),
                        novelty_score: Some(0.4),
                        indexed_at: Some(Utc::now()),
                        invalidated_at: None,
                        deleted_at: None,
                    })
                    .await
                    .expect("insert scoped vector entry");
            }
        }

        let invalidated = backend
            .invalidate_trace_vector_entry_for_submission(
                "tenant-alpha",
                submission_id,
                target_vector_entry_id,
            )
            .await
            .expect("invalidate exact vector entry");
        assert_eq!(invalidated, 1);

        let alpha_entries = backend
            .list_trace_vector_entries("tenant-alpha")
            .await
            .expect("list tenant-alpha vectors");
        assert_eq!(alpha_entries.len(), 2);
        assert!(alpha_entries.iter().any(|entry| {
            entry.vector_entry_id == target_vector_entry_id
                && entry.status == TraceVectorEntryStatus::Invalidated
                && entry.invalidated_at.is_some()
        }));
        assert!(alpha_entries.iter().any(|entry| {
            entry.vector_entry_id == sibling_vector_entry_id
                && entry.status == TraceVectorEntryStatus::Active
                && entry.invalidated_at.is_none()
        }));

        let beta_entries = backend
            .list_trace_vector_entries("tenant-beta")
            .await
            .expect("list tenant-beta vectors");
        assert!(
            beta_entries
                .iter()
                .all(|entry| entry.status == TraceVectorEntryStatus::Active)
        );

        let idempotent = backend
            .invalidate_trace_vector_entry_for_submission(
                "tenant-alpha",
                submission_id,
                target_vector_entry_id,
            )
            .await
            .expect("repeat exact vector invalidation");
        assert_eq!(idempotent, 0);
    }

    #[tokio::test]
    async fn libsql_store_preserves_retention_job_scope_and_items() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir.path().join("trace-retention-jobs.db");
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

        let retention_job_id = Uuid::new_v4();
        let mut action_counts = BTreeMap::new();
        action_counts.insert("records_marked_expired".to_string(), 1);
        action_counts.insert("records_marked_purged".to_string(), 1);
        let job = backend
            .upsert_trace_retention_job(TraceRetentionJobWrite {
                tenant_id: "tenant-alpha".to_string(),
                retention_job_id,
                purpose: "test_retention_purge".to_string(),
                dry_run: false,
                status: TraceRetentionJobStatus::Complete,
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
                completed_at: Some(Utc::now()),
            })
            .await
            .expect("insert alpha retention job");
        assert_eq!(job.tenant_id, "tenant-alpha");
        assert_eq!(job.retention_job_id, retention_job_id);
        assert_eq!(job.status, TraceRetentionJobStatus::Complete);
        assert_eq!(job.action_counts, action_counts);
        assert_eq!(job.selected_expired_count, 1);

        let mut item_counts = BTreeMap::new();
        item_counts.insert("object_refs_invalidated".to_string(), 1);
        item_counts.insert("derived_records_invalidated".to_string(), 1);
        item_counts.insert("records_marked_purged".to_string(), 1);
        let item = backend
            .upsert_trace_retention_job_item(TraceRetentionJobItemWrite {
                tenant_id: "tenant-alpha".to_string(),
                retention_job_id,
                submission_id,
                action: TraceRetentionJobItemAction::Purge,
                status: TraceRetentionJobItemStatus::Done,
                reason: "retention_purged".to_string(),
                action_counts: item_counts.clone(),
                verified_at: Some(Utc::now()),
            })
            .await
            .expect("insert alpha retention job item");
        assert_eq!(item.tenant_id, "tenant-alpha");
        assert_eq!(item.submission_id, submission_id);
        assert_eq!(item.action, TraceRetentionJobItemAction::Purge);
        assert_eq!(item.status, TraceRetentionJobItemStatus::Done);
        assert_eq!(item.action_counts, item_counts);

        let jobs = backend
            .list_trace_retention_jobs("tenant-alpha")
            .await
            .expect("list alpha retention jobs");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].retention_job_id, retention_job_id);
        let beta_jobs = backend
            .list_trace_retention_jobs("tenant-beta")
            .await
            .expect("list beta retention jobs");
        assert!(beta_jobs.is_empty());

        let items = backend
            .list_trace_retention_job_items("tenant-alpha", retention_job_id)
            .await
            .expect("list alpha retention job items");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].submission_id, submission_id);
        let beta_items = backend
            .list_trace_retention_job_items("tenant-beta", retention_job_id)
            .await
            .expect("list beta retention job items");
        assert!(beta_items.is_empty());
    }

    #[tokio::test]
    async fn libsql_store_persists_resumable_revocation_propagation_items() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir.path().join("trace-revocation-propagation.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");

        let submission_id = Uuid::new_v4();
        let mut alpha_submission = sample_submission("tenant-alpha", submission_id);
        let trace_id = alpha_submission.trace_id;
        backend
            .upsert_trace_submission(alpha_submission.clone())
            .await
            .expect("insert alpha submission");
        alpha_submission.tenant_id = "tenant-beta".to_string();
        backend
            .upsert_trace_submission(alpha_submission)
            .await
            .expect("insert beta submission");

        let object_ref_id = Uuid::new_v4();
        let alpha_pending = backend
            .upsert_trace_revocation_propagation_item(TraceRevocationPropagationItemWrite {
                tenant_id: "tenant-alpha".to_string(),
                propagation_item_id: Uuid::new_v4(),
                source_submission_id: submission_id,
                target: TraceRevocationPropagationTarget::ObjectRef { object_ref_id },
                action: TraceRevocationPropagationAction::InvalidateMetadata,
                status: TraceRevocationPropagationItemStatus::Pending,
                idempotency_key: format!("{submission_id}:object-ref:{object_ref_id}"),
                reason: "user_revoked_trace".to_string(),
                attempt_count: 0,
                last_error: None,
                next_attempt_at: None,
                completed_at: None,
                evidence_hash: None,
                metadata: BTreeMap::from([(
                    "artifact_kind".to_string(),
                    "submitted_envelope".to_string(),
                )]),
            })
            .await
            .expect("insert pending object-ref propagation item");

        let tomorrow = Utc::now() + chrono::Duration::days(1);
        backend
            .upsert_trace_revocation_propagation_item(TraceRevocationPropagationItemWrite {
                tenant_id: "tenant-alpha".to_string(),
                propagation_item_id: Uuid::new_v4(),
                source_submission_id: submission_id,
                target: TraceRevocationPropagationTarget::CreditSettlement {
                    credit_event_id: Uuid::new_v4(),
                    credit_account_ref: "credit:tenant-alpha:user".to_string(),
                    settlement_state_at_selection: TraceCreditSettlementState::Final,
                },
                action: TraceRevocationPropagationAction::ReverseCreditSettlement,
                status: TraceRevocationPropagationItemStatus::Failed,
                idempotency_key: format!("{submission_id}:credit:settlement"),
                reason: "awaiting_settlement_reversal_retry".to_string(),
                attempt_count: 1,
                last_error: Some("settlement service unavailable".to_string()),
                next_attempt_at: Some(tomorrow),
                completed_at: None,
                evidence_hash: None,
                metadata: BTreeMap::new(),
            })
            .await
            .expect("insert retry-delayed credit propagation item");

        backend
            .upsert_trace_revocation_propagation_item(TraceRevocationPropagationItemWrite {
                tenant_id: "tenant-alpha".to_string(),
                propagation_item_id: Uuid::new_v4(),
                source_submission_id: submission_id,
                target: TraceRevocationPropagationTarget::PhysicalDeleteReceipt {
                    object_ref_id: Some(object_ref_id),
                    object_store: "trace_commons_file_store".to_string(),
                    object_key: "tenant-alpha/submissions/body.json".to_string(),
                    receipt_sha256: "sha256:delete-receipt".to_string(),
                },
                action: TraceRevocationPropagationAction::RecordPhysicalDeleteReceipt,
                status: TraceRevocationPropagationItemStatus::Done,
                idempotency_key: format!("{submission_id}:physical-delete:{object_ref_id}"),
                reason: "object_payload_deleted".to_string(),
                attempt_count: 1,
                last_error: None,
                next_attempt_at: None,
                completed_at: Some(Utc::now()),
                evidence_hash: Some("sha256:delete-receipt".to_string()),
                metadata: BTreeMap::new(),
            })
            .await
            .expect("insert completed physical delete receipt item");

        backend
            .upsert_trace_revocation_propagation_item(TraceRevocationPropagationItemWrite {
                tenant_id: "tenant-beta".to_string(),
                propagation_item_id: Uuid::new_v4(),
                source_submission_id: submission_id,
                target: TraceRevocationPropagationTarget::ExportManifestItem {
                    export_manifest_id: Uuid::new_v4(),
                    source_submission_id: submission_id,
                },
                action: TraceRevocationPropagationAction::InvalidateExportMembership,
                status: TraceRevocationPropagationItemStatus::Pending,
                idempotency_key: format!("{submission_id}:beta:export-item"),
                reason: "other_tenant_pending_item".to_string(),
                attempt_count: 0,
                last_error: None,
                next_attempt_at: None,
                completed_at: None,
                evidence_hash: None,
                metadata: BTreeMap::new(),
            })
            .await
            .expect("insert beta propagation item");

        let alpha_items = backend
            .list_trace_revocation_propagation_items("tenant-alpha", submission_id)
            .await
            .expect("list alpha revocation propagation items");
        assert_eq!(alpha_items.len(), 3);
        assert!(alpha_items.iter().any(
            |item| item.target == TraceRevocationPropagationTarget::ObjectRef { object_ref_id }
        ));
        assert!(alpha_items.iter().any(|item| matches!(
            item.target,
            TraceRevocationPropagationTarget::CreditSettlement { .. }
        )));
        assert!(alpha_items.iter().all(|item| item.trace_id == trace_id));

        let due_now = backend
            .list_due_trace_revocation_propagation_items("tenant-alpha", Utc::now(), 10)
            .await
            .expect("list due alpha propagation items");
        assert_eq!(due_now.len(), 1);
        assert_eq!(
            due_now[0].propagation_item_id,
            alpha_pending.propagation_item_id
        );

        let updated = backend
            .update_trace_revocation_propagation_item_status(
                "tenant-alpha",
                alpha_pending.propagation_item_id,
                TraceRevocationPropagationItemStatusUpdate {
                    status: TraceRevocationPropagationItemStatus::Done,
                    attempt_count: 1,
                    last_error: None,
                    next_attempt_at: None,
                    completed_at: Some(Utc::now()),
                    evidence_hash: Some("sha256:object-ref-invalidated".to_string()),
                },
            )
            .await
            .expect("update alpha propagation item")
            .expect("alpha propagation item exists");
        assert_eq!(updated.status, TraceRevocationPropagationItemStatus::Done);
        assert_eq!(updated.attempt_count, 1);
        assert_eq!(
            updated.evidence_hash.as_deref(),
            Some("sha256:object-ref-invalidated")
        );

        let due_after_update = backend
            .list_due_trace_revocation_propagation_items("tenant-alpha", Utc::now(), 10)
            .await
            .expect("list due alpha propagation items after update");
        assert!(due_after_update.is_empty());

        let beta_due = backend
            .list_due_trace_revocation_propagation_items("tenant-beta", Utc::now(), 10)
            .await
            .expect("list due beta propagation items");
        assert_eq!(beta_due.len(), 1);
        assert_eq!(beta_due[0].tenant_id, "tenant-beta");

        let missing_cross_tenant = backend
            .update_trace_revocation_propagation_item_status(
                "tenant-beta",
                alpha_pending.propagation_item_id,
                TraceRevocationPropagationItemStatusUpdate {
                    status: TraceRevocationPropagationItemStatus::Done,
                    attempt_count: 1,
                    last_error: None,
                    next_attempt_at: None,
                    completed_at: Some(Utc::now()),
                    evidence_hash: None,
                },
            )
            .await
            .expect("cross-tenant update remains scoped");
        assert!(missing_cross_tenant.is_none());
    }

    #[tokio::test]
    async fn libsql_audit_append_rejects_stale_previous_hash_per_tenant() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir.path().join("trace-corpus.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");

        let tenant_id = "tenant-audit-chain";
        let submission_id = Uuid::new_v4();
        backend
            .upsert_trace_submission(sample_submission(tenant_id, submission_id))
            .await
            .expect("insert submission");

        backend
            .append_trace_audit_event(sample_unhashed_audit_event(tenant_id, submission_id))
            .await
            .expect("append DB-native unhashed audit event");
        backend
            .append_trace_audit_event(sample_audit_event(
                tenant_id,
                submission_id,
                "sha256:file-only-predecessor",
                "sha256:first",
            ))
            .await
            .expect("append first mirrored hash-chain segment");
        backend
            .append_trace_audit_event(sample_audit_event(
                tenant_id,
                submission_id,
                "sha256:first",
                "sha256:second",
            ))
            .await
            .expect("append second audit event");

        let stale_append = backend
            .append_trace_audit_event(sample_audit_event(
                tenant_id,
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
            .list_trace_audit_events(tenant_id)
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

        let other_submission_id = Uuid::new_v4();
        backend
            .upsert_trace_submission(sample_submission("tenant-audit-other", other_submission_id))
            .await
            .expect("insert other tenant submission");
        backend
            .append_trace_audit_event(sample_audit_event(
                "tenant-audit-other",
                other_submission_id,
                "sha256:genesis",
                "sha256:first-other-tenant",
            ))
            .await
            .expect("other tenant starts an independent audit chain");
    }

    #[tokio::test]
    async fn libsql_store_persists_trace_export_grants_jobs_and_tenant_scope() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir.path().join("trace-corpus-export-jobs.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");

        let alpha_job_id = Uuid::new_v4();
        let alpha_grant_id = Uuid::new_v4();
        let alpha_manifest_id = Uuid::new_v4();
        let requested_at = DateTime::parse_from_rfc3339("2026-04-27T12:00:00Z")
            .expect("parse requested_at")
            .with_timezone(&Utc);
        let expires_at = requested_at + chrono::Duration::minutes(15);
        let mut metadata = BTreeMap::new();
        metadata.insert("request_id".to_string(), "req-alpha".to_string());
        metadata.insert("slice".to_string(), "0".to_string());

        let grant = backend
            .upsert_trace_export_access_grant(TraceExportAccessGrantWrite {
                tenant_id: "tenant-alpha".to_string(),
                export_job_id: alpha_job_id,
                grant_id: alpha_grant_id,
                caller_principal_ref: "principal:exporter".to_string(),
                requested_dataset_kind: "replay".to_string(),
                purpose: "offline-eval".to_string(),
                max_item_cap: Some(128),
                status: TraceExportAccessGrantStatus::Active,
                requested_at,
                expires_at,
                metadata: metadata.clone(),
            })
            .await
            .expect("upsert export access grant");
        assert_eq!(grant.tenant_id, "tenant-alpha");
        assert_eq!(grant.export_job_id, alpha_job_id);
        assert_eq!(grant.grant_id, alpha_grant_id);
        assert_eq!(grant.status, TraceExportAccessGrantStatus::Active);
        assert_eq!(grant.max_item_cap, Some(128));
        assert_eq!(grant.metadata, metadata);

        let job = backend
            .upsert_trace_export_job(TraceExportJobWrite {
                tenant_id: "tenant-alpha".to_string(),
                export_job_id: alpha_job_id,
                grant_id: alpha_grant_id,
                caller_principal_ref: "principal:exporter".to_string(),
                requested_dataset_kind: "replay".to_string(),
                purpose: "offline-eval".to_string(),
                max_item_cap: Some(128),
                status: TraceExportJobStatus::Queued,
                requested_at,
                started_at: None,
                finished_at: None,
                expires_at,
                result_manifest_id: None,
                item_count: None,
                last_error: None,
                metadata: metadata.clone(),
            })
            .await
            .expect("upsert export job");
        assert_eq!(job.tenant_id, "tenant-alpha");
        assert_eq!(job.export_job_id, alpha_job_id);
        assert_eq!(job.grant_id, alpha_grant_id);
        assert_eq!(job.status, TraceExportJobStatus::Queued);
        assert!(job.started_at.is_none());
        assert!(job.finished_at.is_none());
        assert_eq!(job.result_manifest_id, None);

        let started_at = requested_at + chrono::Duration::seconds(5);
        let finished_at = requested_at + chrono::Duration::seconds(12);
        let updated = backend
            .update_trace_export_job_status(
                "tenant-alpha",
                alpha_job_id,
                TraceExportJobStatusUpdate {
                    status: TraceExportJobStatus::Complete,
                    started_at: Some(started_at),
                    finished_at: Some(finished_at),
                    result_manifest_id: Some(alpha_manifest_id),
                    item_count: Some(42),
                    last_error: None,
                    metadata: metadata.clone(),
                },
            )
            .await
            .expect("update export job status")
            .expect("updated job exists");
        assert_eq!(updated.status, TraceExportJobStatus::Complete);
        assert_eq!(updated.started_at, Some(started_at));
        assert_eq!(updated.finished_at, Some(finished_at));
        assert_eq!(updated.result_manifest_id, Some(alpha_manifest_id));
        assert_eq!(updated.item_count, Some(42));

        let beta_grant_id = Uuid::new_v4();
        backend
            .upsert_trace_export_access_grant(TraceExportAccessGrantWrite {
                tenant_id: "tenant-beta".to_string(),
                export_job_id: alpha_job_id,
                grant_id: beta_grant_id,
                caller_principal_ref: "principal:beta-exporter".to_string(),
                requested_dataset_kind: "benchmark".to_string(),
                purpose: "tenant-beta-eval".to_string(),
                max_item_cap: Some(7),
                status: TraceExportAccessGrantStatus::Active,
                requested_at,
                expires_at,
                metadata: BTreeMap::new(),
            })
            .await
            .expect("upsert beta export access grant");

        backend
            .upsert_trace_export_job(TraceExportJobWrite {
                tenant_id: "tenant-beta".to_string(),
                export_job_id: alpha_job_id,
                grant_id: beta_grant_id,
                caller_principal_ref: "principal:beta-exporter".to_string(),
                requested_dataset_kind: "benchmark".to_string(),
                purpose: "tenant-beta-eval".to_string(),
                max_item_cap: Some(7),
                status: TraceExportJobStatus::Running,
                requested_at,
                started_at: Some(started_at),
                finished_at: None,
                expires_at,
                result_manifest_id: None,
                item_count: Some(3),
                last_error: None,
                metadata: BTreeMap::new(),
            })
            .await
            .expect("upsert same export job id in other tenant");

        let alpha_jobs = backend
            .list_trace_export_jobs("tenant-alpha")
            .await
            .expect("list alpha export jobs");
        assert_eq!(alpha_jobs.len(), 1);
        assert_eq!(alpha_jobs[0].status, TraceExportJobStatus::Complete);
        assert_eq!(alpha_jobs[0].item_count, Some(42));

        let alpha_grants = backend
            .list_trace_export_access_grants("tenant-alpha")
            .await
            .expect("list alpha export grants");
        assert_eq!(alpha_grants.len(), 1);
        assert_eq!(alpha_grants[0].grant_id, alpha_grant_id);

        let beta_jobs = backend
            .list_trace_export_jobs("tenant-beta")
            .await
            .expect("list beta export jobs");
        assert_eq!(beta_jobs.len(), 1);
        assert_eq!(beta_jobs[0].tenant_id, "tenant-beta");
        assert_eq!(beta_jobs[0].status, TraceExportJobStatus::Running);
        assert_eq!(beta_jobs[0].item_count, Some(3));

        assert!(
            backend
                .update_trace_export_job_status(
                    "tenant-gamma",
                    alpha_job_id,
                    TraceExportJobStatusUpdate {
                        status: TraceExportJobStatus::Failed,
                        started_at: None,
                        finished_at: Some(finished_at),
                        result_manifest_id: None,
                        item_count: None,
                        last_error: Some("not found".to_string()),
                        metadata: BTreeMap::new(),
                    },
                )
                .await
                .expect("tenant-scoped update")
                .is_none()
        );
    }

    #[tokio::test]
    async fn libsql_store_persists_tenant_access_grants_and_active_principal_scope() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir.path().join("trace-tenant-access-grants.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");

        let now = DateTime::parse_from_rfc3339("2026-04-27T12:00:00Z")
            .expect("parse now")
            .with_timezone(&Utc);
        let active_grant_id = Uuid::new_v4();
        let expired_grant_id = Uuid::new_v4();
        let revoked_grant_id = Uuid::new_v4();
        let future_grant_id = Uuid::new_v4();
        let mut metadata = BTreeMap::new();
        metadata.insert("issuer_key_mode".to_string(), "managed_eddsa".to_string());
        metadata.insert("hosted_surface".to_string(), "near.com".to_string());

        let active = backend
            .upsert_trace_tenant_access_grant(TraceTenantAccessGrantWrite {
                tenant_id: "tenant-alpha".to_string(),
                grant_id: active_grant_id,
                principal_ref: "principal:hosted-agent".to_string(),
                role: TraceTenantAccessGrantRole::Contributor,
                status: TraceTenantAccessGrantStatus::Active,
                allowed_consent_scopes: vec![
                    "debugging_evaluation".to_string(),
                    "ranking_training".to_string(),
                ],
                allowed_uses: vec![
                    "debugging_evaluation".to_string(),
                    "ranking_model_training".to_string(),
                ],
                issuer: Some("https://issuer.near.com".to_string()),
                audience: Some("trace-commons".to_string()),
                subject: Some("tenant-alpha-agent".to_string()),
                issued_at: now - chrono::Duration::minutes(5),
                expires_at: Some(now + chrono::Duration::minutes(30)),
                revoked_at: None,
                created_by_principal_ref: Some("issuer:near.com".to_string()),
                revoked_by_principal_ref: None,
                reason: Some("hosted tenant verified".to_string()),
                metadata: metadata.clone(),
            })
            .await
            .expect("insert active tenant access grant");
        assert_eq!(active.tenant_id, "tenant-alpha");
        assert_eq!(active.grant_id, active_grant_id);
        assert_eq!(active.role, TraceTenantAccessGrantRole::Contributor);
        assert_eq!(active.status, TraceTenantAccessGrantStatus::Active);
        assert_eq!(active.metadata, metadata);

        for (grant_id, status, issued_at, expires_at, revoked_at, reason) in [
            (
                expired_grant_id,
                TraceTenantAccessGrantStatus::Active,
                now - chrono::Duration::hours(2),
                Some(now - chrono::Duration::minutes(1)),
                None,
                "expired grant",
            ),
            (
                revoked_grant_id,
                TraceTenantAccessGrantStatus::Revoked,
                now - chrono::Duration::hours(1),
                Some(now + chrono::Duration::minutes(30)),
                Some(now - chrono::Duration::minutes(2)),
                "tenant deprovisioned",
            ),
            (
                future_grant_id,
                TraceTenantAccessGrantStatus::Active,
                now + chrono::Duration::minutes(5),
                Some(now + chrono::Duration::hours(1)),
                None,
                "future activation",
            ),
        ] {
            backend
                .upsert_trace_tenant_access_grant(TraceTenantAccessGrantWrite {
                    tenant_id: "tenant-alpha".to_string(),
                    grant_id,
                    principal_ref: "principal:hosted-agent".to_string(),
                    role: TraceTenantAccessGrantRole::Contributor,
                    status,
                    allowed_consent_scopes: vec!["debugging_evaluation".to_string()],
                    allowed_uses: vec!["debugging_evaluation".to_string()],
                    issuer: Some("https://issuer.near.com".to_string()),
                    audience: Some("trace-commons".to_string()),
                    subject: Some("tenant-alpha-agent".to_string()),
                    issued_at,
                    expires_at,
                    revoked_at,
                    created_by_principal_ref: Some("issuer:near.com".to_string()),
                    revoked_by_principal_ref: revoked_at.map(|_| "admin:alpha".to_string()),
                    reason: Some(reason.to_string()),
                    metadata: BTreeMap::new(),
                })
                .await
                .expect("insert inactive tenant access grant");
        }

        backend
            .upsert_trace_tenant_access_grant(TraceTenantAccessGrantWrite {
                tenant_id: "tenant-beta".to_string(),
                grant_id: active_grant_id,
                principal_ref: "principal:hosted-agent".to_string(),
                role: TraceTenantAccessGrantRole::Admin,
                status: TraceTenantAccessGrantStatus::Active,
                allowed_consent_scopes: vec!["debugging_evaluation".to_string()],
                allowed_uses: vec!["debugging_evaluation".to_string()],
                issuer: Some("https://issuer.near.com".to_string()),
                audience: Some("trace-commons".to_string()),
                subject: Some("tenant-beta-agent".to_string()),
                issued_at: now - chrono::Duration::minutes(5),
                expires_at: Some(now + chrono::Duration::minutes(30)),
                revoked_at: None,
                created_by_principal_ref: Some("issuer:near.com".to_string()),
                revoked_by_principal_ref: None,
                reason: Some("beta grant with same id".to_string()),
                metadata: BTreeMap::new(),
            })
            .await
            .expect("insert same grant id for beta tenant");

        let alpha_grants = backend
            .list_trace_tenant_access_grants("tenant-alpha")
            .await
            .expect("list alpha tenant access grants");
        assert_eq!(alpha_grants.len(), 4);
        assert!(
            alpha_grants
                .iter()
                .all(|grant| grant.tenant_id == "tenant-alpha")
        );

        let active_for_principal = backend
            .list_active_trace_tenant_access_grants_for_principal(
                "tenant-alpha",
                "principal:hosted-agent",
                now,
            )
            .await
            .expect("list active tenant access grants for principal");
        assert_eq!(active_for_principal.len(), 1);
        assert_eq!(active_for_principal[0].grant_id, active_grant_id);
        assert_eq!(
            active_for_principal[0].allowed_uses,
            vec!["debugging_evaluation", "ranking_model_training"]
        );

        let beta_grants = backend
            .list_trace_tenant_access_grants("tenant-beta")
            .await
            .expect("list beta tenant access grants");
        assert_eq!(beta_grants.len(), 1);
        assert_eq!(beta_grants[0].grant_id, active_grant_id);
        assert_eq!(beta_grants[0].role, TraceTenantAccessGrantRole::Admin);
    }
}
