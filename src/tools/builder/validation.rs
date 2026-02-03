//! WASM tool validation.
//!
//! Validates that built WASM modules conform to the expected tool interface
//! before they can be registered with the agent.

use std::path::Path;

use thiserror::Error;

/// Errors during WASM validation.
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Failed to read WASM file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Invalid WASM module: {0}")]
    InvalidModule(String),

    #[error("Missing required export: {0}")]
    MissingExport(String),

    #[error("Invalid export signature for '{name}': expected {expected}, got {actual}")]
    InvalidSignature {
        name: String,
        expected: String,
        actual: String,
    },

    #[error("Module uses disallowed import: {module}::{name}")]
    DisallowedImport { module: String, name: String },

    #[error("Module exceeds size limit: {size} bytes (max: {max} bytes)")]
    TooLarge { size: u64, max: u64 },

    #[error("Validation failed: {0}")]
    Other(String),
}

/// Result of WASM validation.
#[derive(Debug)]
pub struct ValidationResult {
    /// Whether the module is valid.
    pub is_valid: bool,
    /// List of validation errors (empty if valid).
    pub errors: Vec<ValidationError>,
    /// List of warnings (non-fatal issues).
    pub warnings: Vec<String>,
    /// Detected exports.
    pub exports: Vec<ExportInfo>,
    /// Detected imports.
    pub imports: Vec<ImportInfo>,
    /// Module size in bytes.
    pub size_bytes: u64,
}

/// Information about an exported function.
#[derive(Debug, Clone)]
pub struct ExportInfo {
    pub name: String,
    pub kind: ExportKind,
}

/// Kind of export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    Function,
    Memory,
    Table,
    Global,
}

/// Information about an imported function.
#[derive(Debug, Clone)]
pub struct ImportInfo {
    pub module: String,
    pub name: String,
    pub kind: ImportKind,
}

/// Kind of import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    Function,
    Memory,
    Table,
    Global,
}

/// Validator for WASM tool modules.
pub struct WasmValidator {
    /// Maximum module size in bytes.
    max_size: u64,
    /// Required exports that must be present.
    required_exports: Vec<String>,
    /// Allowed import modules.
    allowed_import_modules: Vec<String>,
}

impl Default for WasmValidator {
    fn default() -> Self {
        Self {
            max_size: 10 * 1024 * 1024, // 10 MB
            required_exports: vec!["run".to_string()],
            allowed_import_modules: vec![
                "env".to_string(),
                "wasi_snapshot_preview1".to_string(),
                "wasi".to_string(),
            ],
        }
    }
}

impl WasmValidator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum module size.
    pub fn with_max_size(mut self, max_bytes: u64) -> Self {
        self.max_size = max_bytes;
        self
    }

    /// Add a required export.
    pub fn with_required_export(mut self, name: impl Into<String>) -> Self {
        self.required_exports.push(name.into());
        self
    }

    /// Add an allowed import module.
    pub fn with_allowed_import(mut self, module: impl Into<String>) -> Self {
        self.allowed_import_modules.push(module.into());
        self
    }

    /// Validate a WASM file.
    pub async fn validate_file(&self, path: &Path) -> Result<ValidationResult, ValidationError> {
        let bytes = tokio::fs::read(path).await?;
        self.validate_bytes(&bytes)
    }

    /// Validate WASM bytes.
    pub fn validate_bytes(&self, bytes: &[u8]) -> Result<ValidationResult, ValidationError> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut exports = Vec::new();
        let mut imports = Vec::new();
        let size_bytes = bytes.len() as u64;

        // Check size
        if size_bytes > self.max_size {
            errors.push(ValidationError::TooLarge {
                size: size_bytes,
                max: self.max_size,
            });
        }

        // Parse WASM module
        let parser = wasmparser::Parser::new(0);

        for payload in parser.parse_all(bytes) {
            match payload {
                Ok(wasmparser::Payload::ExportSection(reader)) => {
                    for export in reader {
                        match export {
                            Ok(exp) => {
                                let kind = match exp.kind {
                                    wasmparser::ExternalKind::Func => ExportKind::Function,
                                    wasmparser::ExternalKind::Memory => ExportKind::Memory,
                                    wasmparser::ExternalKind::Table => ExportKind::Table,
                                    wasmparser::ExternalKind::Global => ExportKind::Global,
                                    wasmparser::ExternalKind::Tag => continue,
                                };
                                exports.push(ExportInfo {
                                    name: exp.name.to_string(),
                                    kind,
                                });
                            }
                            Err(e) => {
                                errors.push(ValidationError::InvalidModule(format!(
                                    "Failed to parse export: {}",
                                    e
                                )));
                            }
                        }
                    }
                }
                Ok(wasmparser::Payload::ImportSection(reader)) => {
                    for import in reader {
                        match import {
                            Ok(imp) => {
                                let kind = match imp.ty {
                                    wasmparser::TypeRef::Func(_) => ImportKind::Function,
                                    wasmparser::TypeRef::Memory(_) => ImportKind::Memory,
                                    wasmparser::TypeRef::Table(_) => ImportKind::Table,
                                    wasmparser::TypeRef::Global(_) => ImportKind::Global,
                                    wasmparser::TypeRef::Tag(_) => continue,
                                };

                                imports.push(ImportInfo {
                                    module: imp.module.to_string(),
                                    name: imp.name.to_string(),
                                    kind,
                                });

                                // Check if import module is allowed
                                if !self
                                    .allowed_import_modules
                                    .contains(&imp.module.to_string())
                                {
                                    errors.push(ValidationError::DisallowedImport {
                                        module: imp.module.to_string(),
                                        name: imp.name.to_string(),
                                    });
                                }
                            }
                            Err(e) => {
                                errors.push(ValidationError::InvalidModule(format!(
                                    "Failed to parse import: {}",
                                    e
                                )));
                            }
                        }
                    }
                }
                Ok(_) => {
                    // Other sections are OK
                }
                Err(e) => {
                    errors.push(ValidationError::InvalidModule(format!(
                        "Failed to parse WASM: {}",
                        e
                    )));
                    break;
                }
            }
        }

        // Check required exports
        for required in &self.required_exports {
            if !exports.iter().any(|e| &e.name == required) {
                errors.push(ValidationError::MissingExport(required.clone()));
            }
        }

        // Check for common issues (warnings)
        if !exports
            .iter()
            .any(|e| e.name == "memory" && e.kind == ExportKind::Memory)
        {
            warnings
                .push("Module does not export memory - host cannot read/write data".to_string());
        }

        // Check for potentially dangerous imports
        for import in &imports {
            if import.module == "wasi_snapshot_preview1" {
                match import.name.as_str() {
                    "fd_write" | "fd_read" | "path_open" | "path_create_directory" => {
                        warnings.push(format!(
                            "Module uses WASI filesystem function '{}' - ensure this is intended",
                            import.name
                        ));
                    }
                    "sock_send" | "sock_recv" | "sock_accept" => {
                        warnings.push(format!(
                            "Module uses WASI socket function '{}' - ensure this is intended",
                            import.name
                        ));
                    }
                    _ => {}
                }
            }
        }

        Ok(ValidationResult {
            is_valid: errors.is_empty(),
            errors,
            warnings,
            exports,
            imports,
            size_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validator_default() {
        let validator = WasmValidator::new();
        assert_eq!(validator.max_size, 10 * 1024 * 1024);
        assert!(validator.required_exports.contains(&"run".to_string()));
    }

    #[test]
    fn test_validator_builder() {
        let validator = WasmValidator::new()
            .with_max_size(1024)
            .with_required_export("custom_export")
            .with_allowed_import("custom_module");

        assert_eq!(validator.max_size, 1024);
        assert!(
            validator
                .required_exports
                .contains(&"custom_export".to_string())
        );
        assert!(
            validator
                .allowed_import_modules
                .contains(&"custom_module".to_string())
        );
    }

    // Note: Full WASM parsing tests would require actual WASM binaries
}
