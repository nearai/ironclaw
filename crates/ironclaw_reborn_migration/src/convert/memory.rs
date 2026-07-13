//! Memory / workspace document converter (v1 `memory_documents` → Reborn
//! `ironclaw_memory` documents).
//!
//! Each non-engine v1 document is written through the memory service under the
//! migrated (tenant, user, agent) scope; content and path are preserved.
//! Engine-v2 documents (mission/project/runtime blobs) are skipped here:
//! supported missions and projects are handled by their owning converters,
//! while other runtime blobs remain unsupported. Chunks/embeddings are derived
//! state the memory service recomputes on write, so they are not migrated (not
//! a loss). Version history (`memory_document_versions`) has no Reborn target
//! and is recorded as a loss.

use ironclaw_host_api::{CorrelationId, InvocationId, ResourceScope};
use ironclaw_memory::{
    DocumentMetadata, MemoryInvocation, MemoryServiceErrorKind, MemoryServiceReadRequest,
    MemoryServiceWriteRequest,
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
        let docs = src.all_memory_documents(user_id).await?;

        for doc in docs {
            if v2_model::is_engine_path(&doc.path) {
                if !has_engine_converter(&doc.path) {
                    report.record_loss(
                        Domain::Memory,
                        doc.path,
                        "*",
                        LossReason::NoTargetConcept,
                        "engine-v2 runtime document has no supported Reborn converter and was not migrated",
                    );
                }
                continue;
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
            match tgt
                .memory_service
                .read(
                    invocation.clone(),
                    MemoryServiceReadRequest {
                        path: doc.path.clone(),
                    },
                )
                .await
            {
                Ok(existing) if existing.content == doc.content => {
                    let existing_metadata = tgt
                        .memory_service
                        .read_metadata(
                            invocation.clone(),
                            MemoryServiceReadRequest {
                                path: doc.path.clone(),
                            },
                        )
                        .await
                        .map_err(|error| MigrationError::WriteTarget {
                            domain: format!("memory document {}", doc.path),
                            reason: format!("read deterministic target metadata: {error}"),
                        })?
                        .metadata
                        .unwrap_or_default();
                    if existing_metadata == metadata {
                        report.stats.memory_documents += 1;
                        continue;
                    }
                    return Err(MigrationError::WriteTarget {
                        domain: format!("memory document {}", doc.path),
                        reason: "deterministic memory path already contains divergent metadata; refusing to overwrite"
                            .to_string(),
                    });
                }
                Ok(_) => {
                    return Err(MigrationError::WriteTarget {
                        domain: format!("memory document {}", doc.path),
                        reason: "deterministic memory path already contains divergent content; refusing to overwrite"
                            .to_string(),
                    });
                }
                // Native memory reports a missing document as an input error.
                // The subsequent write remains the authoritative path
                // validation, so malformed source paths still fail closed.
                Err(error) if error.kind() == MemoryServiceErrorKind::Input => {}
                Err(error) => {
                    return Err(MigrationError::WriteTarget {
                        domain: format!("memory document {}", doc.path),
                        reason: format!("read deterministic target slot: {error}"),
                    });
                }
            }
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

fn has_engine_converter(path: &str) -> bool {
    if path.ends_with("mission.json") || (path.contains("/threads/") && path.ends_with(".json")) {
        return true;
    }
    let segments: Vec<_> = path.trim_matches('/').split('/').collect();
    matches!(
        segments.as_slice(),
        ["engine", "projects", slug, "project.json"]
            | [".system", "engine", "projects", slug, "project.json"]
            if !slug.is_empty()
    )
}

#[cfg(test)]
mod tests {
    use super::has_engine_converter;

    #[test]
    fn only_engine_documents_owned_by_specialized_converters_are_recognized() {
        assert!(has_engine_converter("engine/missions/daily/mission.json"));
        assert!(has_engine_converter(".system/engine/threads/id.json"));
        assert!(has_engine_converter("engine/projects/demo/project.json"));
        assert!(!has_engine_converter("engine/runtime/checkpoint.json"));
        assert!(!has_engine_converter("engine/projects/project.json"));
    }
}
