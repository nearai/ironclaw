use thiserror::Error;

/// Errors returned by the Reborn WASM runtime.
#[derive(Debug, Error)]
pub enum WasmError {
    #[error("failed to create WASM engine: {0}")]
    EngineCreationFailed(String),
    #[error("failed to compile WIT component: {0}")]
    CompilationFailed(String),
    #[error("failed to configure WASM store: {0}")]
    StoreConfiguration(String),
    #[error("failed to configure WASM linker: {0}")]
    LinkerConfiguration(String),
    #[error("failed to instantiate WIT component: {0}")]
    InstantiationFailed(String),
    #[error("failed to execute WIT component: {0}")]
    ExecutionFailed(String),
    #[error("tool schema export did not return a valid JSON object: {0}")]
    InvalidSchema(String),
}

/// Errors returned by injected host services.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WasmHostError {
    #[error("{0}")]
    Denied(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Failed(String),
    #[error("{0}")]
    FailedAfterRequestSent(String),
}

impl WasmHostError {
    pub(crate) fn request_was_sent(&self) -> bool {
        matches!(self, Self::FailedAfterRequestSent(_))
    }
}
