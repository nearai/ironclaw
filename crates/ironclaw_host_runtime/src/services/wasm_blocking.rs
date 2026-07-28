use std::sync::Arc;

use static_assertions::const_assert;
use tokio::sync::Semaphore;

use super::runtime_adapters::wasm_error_kind;
use super::{
    PreparedWitTool, RuntimeDispatchErrorKind, WasmError, WitToolExecution, WitToolHost,
    WitToolRequest, WitToolRuntime,
};

/// Upper bound on concurrent native WASM executions.
pub(super) const MAX_CONCURRENT_WASM_EXEC: usize = 64;

const_assert!(MAX_CONCURRENT_WASM_EXEC > 0 && MAX_CONCURRENT_WASM_EXEC < 512);

/// Upper bound on concurrent WASM component compilations.
pub(super) const MAX_CONCURRENT_WASM_PREPARE: usize = 16;

const_assert!(
    MAX_CONCURRENT_WASM_PREPARE > 0 && MAX_CONCURRENT_WASM_PREPARE < MAX_CONCURRENT_WASM_EXEC
);

/// Process-wide gate over concurrent native WASM execution.
pub(super) static WASM_EXEC_SEMAPHORE: std::sync::LazyLock<Arc<Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_WASM_EXEC)));

/// Process-wide gate over concurrent WASM component compilation.
pub(super) static WASM_PREPARE_SEMAPHORE: std::sync::LazyLock<Arc<Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_WASM_PREPARE)));

/// Failure returned by the host-owned blocking wrapper around WASM work.
///
/// `WasmError::ExecutionFailed` can describe either a guest execution trap or
/// a host failure while acquiring or joining the blocking task. This wrapper
/// preserves that provenance for dispatch classification.
#[derive(Debug)]
pub(super) struct WasmBlockingError {
    source: WasmError,
    kind: RuntimeDispatchErrorKind,
}

impl WasmBlockingError {
    fn runtime(source: WasmError) -> Self {
        let kind = wasm_error_kind(&source);
        Self { source, kind }
    }

    fn executor(message: impl Into<String>) -> Self {
        Self {
            source: WasmError::execution_failed(message.into()),
            kind: RuntimeDispatchErrorKind::Executor,
        }
    }

    pub(super) fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }

    pub(super) fn source(&self) -> &WasmError {
        &self.source
    }
}

/// Run a synchronous wasmtime guest call on the bounded blocking pool.
pub(super) async fn run_wasm_execution_blocking(
    runtime: WitToolRuntime,
    prepared: Arc<PreparedWitTool>,
    host: WitToolHost,
    input_json: String,
    context_json: String,
) -> Result<WitToolExecution, WasmBlockingError> {
    let permit = WASM_EXEC_SEMAPHORE
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| WasmBlockingError::executor("wasm execution gate closed"))?;
    let execution = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        runtime.execute(
            &prepared,
            host,
            WitToolRequest::new(input_json).with_context(context_json),
        )
    })
    .await
    .map_err(|_| WasmBlockingError::executor("wasm execution task panicked"))?;
    execution.map_err(WasmBlockingError::runtime)
}

/// Run synchronous wasmtime component compilation on the bounded blocking pool.
pub(super) async fn run_wasm_prepare_blocking(
    runtime: WitToolRuntime,
    package_id: String,
    wasm_bytes: Vec<u8>,
) -> Result<PreparedWitTool, WasmBlockingError> {
    let permit = WASM_PREPARE_SEMAPHORE
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| WasmBlockingError::executor("wasm preparation gate closed"))?;
    let prepared = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        runtime.prepare(&package_id, &wasm_bytes)
    })
    .await
    .map_err(|_| WasmBlockingError::executor("wasm preparation task panicked"))?;
    prepared.map_err(WasmBlockingError::runtime)
}
