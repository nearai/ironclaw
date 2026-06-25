//! JSON schema validation for memory document content.

use ironclaw_filesystem::{FilesystemError, FilesystemOperation};

use crate::path::{MemoryDocumentPath, memory_error, valid_memory_path};

pub(crate) fn validate_content_against_schema(
    path: &MemoryDocumentPath,
    content: &str,
    schema: &serde_json::Value,
) -> Result<(), FilesystemError> {
    if schema.is_null() {
        return Ok(());
    }
    let instance: serde_json::Value = serde_json::from_str(content).map_err(|error| {
        memory_error(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::WriteFile,
            format!("schema validation failed: content is not valid JSON: {error}"),
        )
    })?;
    let validator = jsonschema::validator_for(schema).map_err(|error| {
        memory_error(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::WriteFile,
            format!("schema validation failed: invalid schema: {error}"),
        )
    })?;
    let errors = validator
        .iter_errors(&instance)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(memory_error(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::WriteFile,
            format!("schema validation failed: {}", errors.join("; ")),
        ))
    }
}
