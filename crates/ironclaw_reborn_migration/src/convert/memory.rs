//! Memory / workspace document converter (v1 `memory_documents` → Reborn
//! `ironclaw_memory` documents).
//!
//! Each non-engine v1 document is written through the memory service under the
//! migrated (tenant, user, agent) scope; content and path are preserved.
//! Engine-v2 documents (mission/project/runtime blobs) are skipped here — they
//! are consumed by the automations converter. Chunks/embeddings are derived
//! state the memory service recomputes on write, so they are not migrated (not
//! a loss). Version history (`memory_document_versions`) has no Reborn target
//! and is recorded as a loss.

use ironclaw_host_api::{CorrelationId, InvocationId, ResourceScope};
use ironclaw_memory::{DocumentMetadata, MemoryInvocation};
use ironclaw_memory_native::{
    MemoryBackendWriteOptions, MemoryContext, MemoryDocumentPath, MemoryDocumentScope,
};

use crate::error::MigrationError;
use crate::options::MigrationOptions;
use crate::report::{Domain, LossReason, MigrationReport};
use crate::source::V1Source;
use crate::target::RebornTarget;
use crate::v2_model;

pub(crate) async fn run(
    src: &V1Source,
    tgt: &mut RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let users = src.distinct_users().await?;
    for user_id in &users {
        let docs =
            src.db
                .list_documents(user_id, None)
                .await
                .map_err(|e| MigrationError::ReadSource {
                    domain: "memory_documents".into(),
                    reason: e.to_string(),
                })?;

        for doc in docs {
            if v2_model::is_engine_path(&doc.path) {
                continue; // engine-v2 state — handled by the automations converter
            }

            // A malformed source user id is a per-item loss, not a run abort.
            let Some(user) =
                report.valid_user_id(Domain::Memory, doc.path.clone(), "user_id", &doc.user_id)
            else {
                continue;
            };

            if options.dry_run {
                report.stats.memory_documents += 1;
                continue;
            }

            let scope = ResourceScope {
                tenant_id: tgt.tenant_id.clone(),
                user_id: user,
                agent_id: Some(tgt.agent_id.clone()),
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            };
            let invocation = MemoryInvocation {
                scope: scope.clone(),
                correlation_id: CorrelationId::new(),
            };
            let metadata = DocumentMetadata::from_value(&doc.metadata);
            let memory_scope = match MemoryDocumentScope::new_with_agent(
                invocation.scope.tenant_id.as_str(),
                invocation.scope.user_id.as_str(),
                invocation.scope.agent_id.as_ref().map(|id| id.as_str()),
                invocation.scope.project_id.as_ref().map(|id| id.as_str()),
            ) {
                Ok(scope) => scope,
                Err(error) => {
                    report.record_loss(
                        Domain::Memory,
                        memory_source_id(&doc.user_id, &doc.path),
                        "scope",
                        LossReason::Unparseable,
                        format!("legacy memory document scope is not a valid Reborn memory scope (record skipped): {error}"),
                    );
                    continue;
                }
            };
            let path = match MemoryDocumentPath::from_scope(memory_scope.clone(), doc.path.clone())
            {
                Ok(path) => path,
                Err(error) => {
                    report.record_loss(
                        Domain::Memory,
                        memory_source_id(&doc.user_id, &doc.path),
                        "path",
                        LossReason::Unparseable,
                        format!("legacy memory document path is not a valid Reborn memory path (record skipped): {error}"),
                    );
                    continue;
                }
            };
            let context = MemoryContext::new(memory_scope)
                .with_audit_context(invocation.scope, invocation.correlation_id);
            let write_options = MemoryBackendWriteOptions::with_metadata_overlay(Some(metadata));

            match tgt
                .memory_backend
                .write_document_with_backend_options(
                    &context,
                    &path,
                    doc.content.as_bytes(),
                    &write_options,
                )
                .await
            {
                Ok(_) => {
                    report.stats.memory_documents += 1;
                }
                Err(error) => {
                    return Err(MigrationError::WriteTarget {
                        domain: format!("memory document {}", doc.path),
                        reason: error.to_string(),
                    });
                }
            }
        }
    }

    report.record_loss(
        Domain::Memory,
        "memory_document_versions",
        "*",
        LossReason::NoTargetConcept,
        "Reborn memory has no per-document version/undo history; v1 \
         memory_document_versions rows are not migrated"
            .to_string(),
    );
    Ok(())
}

fn memory_source_id(user_id: &str, path: &str) -> String {
    format!("{user_id}:{path}")
}
