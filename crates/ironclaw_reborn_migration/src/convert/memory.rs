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
use ironclaw_memory::{DocumentMetadata, MemoryInvocation, MemoryServiceWriteRequest};

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
                scope,
                correlation_id: CorrelationId::new(),
            };
            let metadata = DocumentMetadata::from_value(&doc.metadata);
            let request = MemoryServiceWriteRequest {
                target: doc.path.clone(),
                content: doc.content.clone(),
                append: false,
                old_string: None,
                new_string: None,
                replace_all: false,
                metadata: Some(metadata),
                timezone: None,
            };

            tgt.memory_service
                .write(invocation, request)
                .await
                .map_err(|e| MigrationError::WriteTarget {
                    domain: format!("memory document {}", doc.path),
                    reason: e.to_string(),
                })?;
            report.stats.memory_documents += 1;
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
