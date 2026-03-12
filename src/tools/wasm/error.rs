//! WASM sandbox error types.

use thiserror::Error;

/// Errors that can occur during WASM tool execution.
#[derive(Debug, Error)]
pub enum WasmError {
    /// Failed to create the Wasmtime engine.
    #[error("Engine creation failed: {0}")]
    EngineCreationFailed(String),

    /// Failed to compile WASM bytes into a component.
    #[error("Compilation failed: {0}")]
    CompilationFailed(String),

    /// WASM validation failed (malformed or invalid component).
    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    /// Failed to instantiate the component.
    #[error("Instantiation failed: {0}")]
    InstantiationFailed(String),

    /// Component execution trapped (e.g., unreachable, memory access violation).
    #[error("Execution trapped: {0}")]
    Trapped(String),

    /// Component panicked during execution.
    #[error("Execution panicked: {0}")]
    ExecutionPanicked(String),

    /// Fuel limit exhausted during execution.
    #[error("Fuel exhausted: execution exceeded {limit} fuel units")]
    FuelExhausted {
        /// The fuel limit that was exceeded.
        limit: u64,
    },

    /// Memory limit exceeded during execution.
    #[error("Memory limit exceeded: {used} bytes used, {limit} bytes allowed")]
    MemoryExceeded {
        /// Bytes used when limit was hit.
        used: u64,
        /// Maximum allowed bytes.
        limit: u64,
    },

    /// Required export not found in component.
    #[error("Missing export: {0}")]
    MissingExport(String),

    /// IO error (e.g., reading WASM file).
    #[error("IO error: {0}")]
    IoError(String),

    /// Configuration error.
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Host function error.
    #[error("Host error: {0}")]
    HostError(String),

    /// Execution timed out.
    #[error("Execution timed out after {0:?}")]
    Timeout(std::time::Duration),

    /// Component returned an error response.
    /// When `hint` is non-empty it carries the tool's description and parameter
    /// schema so the LLM can retry with correct arguments.
    #[error("Tool error: {message}{}", if hint.is_empty() { String::new() } else { format!("\n\nTool usage hint:\n{hint}") })]
    ToolReturnedError {
        /// The error message from the WASM tool.
        message: String,
        /// Optional description + schema hint (empty when unavailable).
        hint: String,
    },

    /// Invalid JSON in tool response.
    #[error("Invalid response JSON: {0}")]
    InvalidResponseJson(String),

    /// Path traversal attempt blocked.
    #[error("Path traversal blocked: {0}")]
    PathTraversalBlocked(String),
}

impl From<std::io::Error> for WasmError {
    fn from(e: std::io::Error) -> Self {
        WasmError::IoError(e.to_string())
    }
}

impl From<WasmError> for crate::tools::ToolError {
    fn from(e: WasmError) -> Self {
        crate::tools::ToolError::Sandbox(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::wasm::error::WasmError;

    #[test]
    fn test_error_display() {
        let err = WasmError::FuelExhausted { limit: 1_000_000 };
        assert!(err.to_string().contains("1000000"));

        let err = WasmError::MemoryExceeded {
            used: 20_000_000,
            limit: 10_000_000,
        };
        assert!(err.to_string().contains("20000000"));
        assert!(err.to_string().contains("10000000"));
    }

    #[test]
    fn test_conversion_to_tool_error() {
        let wasm_err = WasmError::Trapped("test trap".to_string());
        let tool_err: crate::tools::ToolError = wasm_err.into();
        match tool_err {
            crate::tools::ToolError::Sandbox(msg) => {
                assert!(msg.contains("test trap"));
            }
            _ => panic!("Expected Sandbox variant"),
        }
    }

    #[test]
    fn test_tool_returned_error_without_hint() {
        let err = WasmError::ToolReturnedError {
            message: "unknown action: foobar".to_string(),
            hint: String::new(),
        };
        let display = err.to_string();
        assert!(display.contains("unknown action: foobar"));
        assert!(!display.contains("Tool usage hint"));
    }

    #[test]
    fn test_tool_returned_error_with_hint() {
        let err = WasmError::ToolReturnedError {
            message: "unknown action: foobar".to_string(),
            hint: "Tip: call tool_info(name: \"gmail\", include_schema: true) for the full parameter schema.".to_string(),
        };
        let display = err.to_string();
        assert!(display.contains("unknown action: foobar"));
        assert!(display.contains("Tool usage hint"));
        assert!(display.contains("tool_info"));
    }
}
