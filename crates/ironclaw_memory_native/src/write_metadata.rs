//! Metadata resolution for memory document writes.

use ironclaw_filesystem::FilesystemError;

use crate::metadata::{DocumentMetadata, MemoryBackendWriteOptions, resolve_document_metadata};
use crate::path::MemoryDocumentPath;
use crate::repo::MemoryDocumentRepository;

pub(crate) async fn resolve_write_metadata<R>(
    repository: &R,
    path: &MemoryDocumentPath,
    options: &MemoryBackendWriteOptions,
) -> Result<(DocumentMetadata, Option<serde_json::Value>), FilesystemError>
where
    R: MemoryDocumentRepository + ?Sized,
{
    let effective = resolve_document_metadata(repository, path).await?;
    let Some(overlay) = options.metadata_overlay.as_ref() else {
        return Ok((effective, None));
    };

    let overlay = overlay.to_value();
    let existing_sidecar = repository
        .read_document_metadata(path)
        .await?
        .unwrap_or_else(|| serde_json::json!({}));
    let sidecar_to_persist = DocumentMetadata::merge(&existing_sidecar, &overlay);
    let effective_with_overlay =
        DocumentMetadata::from_value(&DocumentMetadata::merge(&effective.to_value(), &overlay));
    Ok((effective_with_overlay, Some(sidecar_to_persist)))
}
