//! WASM sandbox error types.

use std::fmt;

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
    #[error("Tool error: {0}")]
    ToolReturnedError(String),

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

/// Details about a trap that occurred during execution.
#[derive(Debug, Clone)]
pub struct TrapInfo {
    /// Human-readable trap message.
    pub message: String,
    /// Trap code if available.
    pub code: Option<TrapCode>,
}

impl fmt::Display for TrapInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.code {
            Some(code) => write!(f, "{}: {}", code, self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

/// Known trap codes from Wasmtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapCode {
    /// Out of bounds memory access.
    MemoryOutOfBounds,
    /// Out of bounds table access.
    TableOutOfBounds,
    /// Indirect call type mismatch.
    IndirectCallToNull,
    /// Signature mismatch on indirect call.
    BadSignature,
    /// Integer overflow.
    IntegerOverflow,
    /// Integer division by zero.
    IntegerDivisionByZero,
    /// Invalid conversion to integer.
    BadConversionToInteger,
    /// Unreachable instruction executed.
    UnreachableCodeReached,
    /// Call stack exhausted.
    StackOverflow,
    /// Out of fuel.
    OutOfFuel,
    /// Unknown trap code.
    Unknown,
}

impl fmt::Display for TrapCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            TrapCode::MemoryOutOfBounds => "memory out of bounds",
            TrapCode::TableOutOfBounds => "table out of bounds",
            TrapCode::IndirectCallToNull => "indirect call to null",
            TrapCode::BadSignature => "bad signature",
            TrapCode::IntegerOverflow => "integer overflow",
            TrapCode::IntegerDivisionByZero => "integer division by zero",
            TrapCode::BadConversionToInteger => "bad conversion to integer",
            TrapCode::UnreachableCodeReached => "unreachable code reached",
            TrapCode::StackOverflow => "stack overflow",
            TrapCode::OutOfFuel => "out of fuel",
            TrapCode::Unknown => "unknown trap",
        };
        write!(f, "{}", s)
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::wasm::error::{TrapCode, TrapInfo, WasmError};

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
    fn test_trap_info_display() {
        let info = TrapInfo {
            message: "access at offset 0x1000".to_string(),
            code: Some(TrapCode::MemoryOutOfBounds),
        };
        let s = info.to_string();
        assert!(s.contains("memory out of bounds"));
        assert!(s.contains("access at offset"));
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
}
