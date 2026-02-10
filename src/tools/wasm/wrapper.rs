//! WASM tool wrapper implementing the Tool trait.
//!
//! Each execution creates a fresh instance (NEAR pattern) to ensure
//! isolation and deterministic behavior.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use wasmtime::Store;
use wasmtime::component::{Component, Linker, Val};

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput};
use crate::tools::wasm::capabilities::Capabilities;
use crate::tools::wasm::error::WasmError;
use crate::tools::wasm::host::{HostState, LogLevel};
use crate::tools::wasm::limits::{ResourceLimits, WasmResourceLimiter};
use crate::tools::wasm::runtime::{PreparedModule, WasmToolRuntime};

/// Store data for WASM execution.
///
/// Contains both the resource limiter and host state.
struct StoreData {
    limiter: WasmResourceLimiter,
    host_state: HostState,
}

impl StoreData {
    fn new(memory_limit: u64, capabilities: Capabilities) -> Self {
        Self {
            limiter: WasmResourceLimiter::new(memory_limit),
            host_state: HostState::new(capabilities),
        }
    }
}

/// A Tool implementation backed by a WASM component.
///
/// Each call to `execute` creates a fresh instance for isolation.
pub struct WasmToolWrapper {
    /// Runtime for engine access.
    runtime: Arc<WasmToolRuntime>,
    /// Prepared module with compiled component.
    prepared: Arc<PreparedModule>,
    /// Capabilities to grant to this tool.
    capabilities: Capabilities,
    /// Cached description (from PreparedModule or override).
    description: String,
    /// Cached schema (from PreparedModule or override).
    schema: serde_json::Value,
}

impl WasmToolWrapper {
    /// Create a new WASM tool wrapper.
    pub fn new(
        runtime: Arc<WasmToolRuntime>,
        prepared: Arc<PreparedModule>,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            description: prepared.description.clone(),
            schema: prepared.schema.clone(),
            runtime,
            prepared,
            capabilities,
        }
    }

    /// Override the tool description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Override the parameter schema.
    pub fn with_schema(mut self, schema: serde_json::Value) -> Self {
        self.schema = schema;
        self
    }

    /// Get the resource limits for this tool.
    pub fn limits(&self) -> &ResourceLimits {
        &self.prepared.limits
    }

    /// Execute the WASM tool synchronously (called from spawn_blocking).
    fn execute_sync(
        &self,
        params: serde_json::Value,
        context_json: Option<String>,
    ) -> Result<(String, Vec<crate::tools::wasm::host::LogEntry>), WasmError> {
        let engine = self.runtime.engine();
        let limits = &self.prepared.limits;

        // Create store with fresh state (NEAR pattern: fresh instance per call)
        let store_data = StoreData::new(limits.memory_bytes, self.capabilities.clone());
        let mut store = Store::new(engine, store_data);

        // Configure fuel if enabled
        if self.runtime.config().fuel_config.enabled {
            store
                .set_fuel(limits.fuel)
                .map_err(|e| WasmError::ConfigError(format!("Failed to set fuel: {}", e)))?;
        }

        // Configure epoch deadline for timeout backup
        store.epoch_deadline_trap();
        store.set_epoch_deadline(1);

        // Set up resource limiter
        store.limiter(|data| &mut data.limiter);

        // Compile the component (uses cached bytes)
        let component = Component::new(engine, self.prepared.component_bytes())
            .map_err(|e| WasmError::CompilationFailed(e.to_string()))?;

        // Create linker and add host functions
        let mut linker = Linker::new(engine);
        self.add_host_functions(&mut linker)?;

        // Instantiate the component
        let instance = linker
            .instantiate(&mut store, &component)
            .map_err(|e| WasmError::InstantiationFailed(e.to_string()))?;

        // Get the execute function
        let execute_func = instance
            .get_func(&mut store, "execute")
            .ok_or_else(|| WasmError::MissingExport("execute".to_string()))?;

        // Prepare request
        let params_json = serde_json::to_string(&params)
            .map_err(|e| WasmError::InvalidResponseJson(e.to_string()))?;

        // Build request record
        // Note: The exact calling convention depends on how WIT records are lowered.
        // With component model, we'd use typed bindings from wit-bindgen.
        // For now, we use the lower-level Val API.
        let request_params = Val::String(params_json);
        let request_context = match context_json {
            Some(ctx) => Val::Option(Some(Box::new(Val::String(ctx)))),
            None => Val::Option(None),
        };

        // Create request record (params, context)
        let request = Val::Record(vec![
            ("params".to_string(), request_params),
            ("context".to_string(), request_context),
        ]);

        // Call the function
        let mut results = vec![Val::Bool(false)]; // Placeholder for response
        execute_func
            .call(&mut store, &[request], &mut results)
            .map_err(|e| {
                // Check for specific trap types
                let error_str = e.to_string();
                if error_str.contains("out of fuel") {
                    WasmError::FuelExhausted { limit: limits.fuel }
                } else if error_str.contains("unreachable") {
                    WasmError::Trapped("unreachable code executed".to_string())
                } else {
                    WasmError::Trapped(error_str)
                }
            })?;

        // Post-call completion (cleanup)
        execute_func
            .post_return(&mut store)
            .map_err(|e| WasmError::Trapped(format!("post_return failed: {}", e)))?;

        // Extract response
        let response = &results[0];
        let (result_str, error_str) = extract_response(response)?;

        // Get logs from host state
        let logs = store.data_mut().host_state.take_logs();

        // Check for tool-level error
        if let Some(err) = error_str {
            return Err(WasmError::ToolReturnedError(err));
        }

        // Return result (or empty string if none)
        Ok((result_str.unwrap_or_default(), logs))
    }

    /// Add host functions to the linker.
    fn add_host_functions(&self, linker: &mut Linker<StoreData>) -> Result<(), WasmError> {
        // Note: With WIT bindgen, these would be generated automatically.
        // For now, we manually define the host functions.
        //
        // Component model func_wrap signature: F: Fn(StoreContextMut<T>, Params) -> Result<Return>
        // where Params is a tuple of the function arguments.

        // host.log(level: log-level, message: string)
        linker
            .root()
            .func_wrap(
                "log",
                |mut ctx: wasmtime::StoreContextMut<'_, StoreData>,
                 (level, message): (i32, String)| {
                    let log_level = match level {
                        0 => LogLevel::Trace,
                        1 => LogLevel::Debug,
                        2 => LogLevel::Info,
                        3 => LogLevel::Warn,
                        4 => LogLevel::Error,
                        _ => LogLevel::Info,
                    };
                    // Ignore errors from logging (rate limiting)
                    let _ = ctx.data_mut().host_state.log(log_level, message);
                    Ok(())
                },
            )
            .map_err(|e| WasmError::ConfigError(format!("Failed to add log function: {}", e)))?;

        // host.now-millis() -> u64
        linker
            .root()
            .func_wrap(
                "now-millis",
                |ctx: wasmtime::StoreContextMut<'_, StoreData>, (): ()| -> anyhow::Result<(u64,)> {
                    Ok((ctx.data().host_state.now_millis(),))
                },
            )
            .map_err(|e| {
                WasmError::ConfigError(format!("Failed to add now-millis function: {}", e))
            })?;

        // host.workspace-read(path: string) -> option<string>
        linker
            .root()
            .func_wrap(
                "workspace-read",
                |ctx: wasmtime::StoreContextMut<'_, StoreData>,
                 (path,): (String,)|
                 -> anyhow::Result<(Option<String>,)> {
                    let result = ctx.data().host_state.workspace_read(&path).ok().flatten();
                    Ok((result,))
                },
            )
            .map_err(|e| {
                WasmError::ConfigError(format!("Failed to add workspace-read function: {}", e))
            })?;

        // host.sign-payload(key-label, payload, context-json) -> sign-result
        // Returns a record { signature: option<string>, error: option<string>, approval-pending: bool }
        linker
            .root()
            .func_wrap(
                "sign-payload",
                |mut ctx: wasmtime::StoreContextMut<'_, StoreData>,
                 (key_label, payload, context_json): (String, String, String)|
                 -> anyhow::Result<(Option<String>, Option<String>, bool)> {
                    let result =
                        ctx.data_mut()
                            .host_state
                            .sign_payload(&key_label, &payload, &context_json);
                    Ok((result.signature, result.error, result.approval_pending))
                },
            )
            .map_err(|e| {
                WasmError::ConfigError(format!("Failed to add sign-payload function: {}", e))
            })?;

        Ok(())
    }
}

/// Extract result and error from a WIT response record.
fn extract_response(response: &Val) -> Result<(Option<String>, Option<String>), WasmError> {
    match response {
        Val::Record(fields) => {
            let mut result = None;
            let mut error = None;

            for (name, val) in fields {
                match name.as_str() {
                    "output" => {
                        if let Val::Option(Some(inner)) = val {
                            if let Val::String(s) = inner.as_ref() {
                                result = Some(s.to_string());
                            }
                        }
                    }
                    "error" => {
                        if let Val::Option(Some(inner)) = val {
                            if let Val::String(s) = inner.as_ref() {
                                error = Some(s.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }

            Ok((result, error))
        }
        _ => Err(WasmError::InvalidResponseJson(
            "Expected record response".to_string(),
        )),
    }
}

#[async_trait]
impl Tool for WasmToolWrapper {
    fn name(&self) -> &str {
        &self.prepared.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let timeout = self.prepared.limits.timeout;

        // Serialize context for WASM
        let context_json = serde_json::to_string(ctx).ok();

        // Clone what we need for the blocking task
        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = self.capabilities.clone();
        let description = self.description.clone();
        let schema = self.schema.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            let wrapper = WasmToolWrapper {
                runtime,
                prepared,
                capabilities,
                description,
                schema,
            };

            tokio::task::spawn_blocking(move || wrapper.execute_sync(params, context_json))
                .await
                .map_err(|e| WasmError::ExecutionPanicked(e.to_string()))?
        })
        .await;

        let duration = start.elapsed();

        match result {
            Ok(Ok((result_json, logs))) => {
                // Emit collected logs
                for log in logs {
                    match log.level {
                        LogLevel::Trace => tracing::trace!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Debug => tracing::debug!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Info => tracing::info!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Warn => tracing::warn!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Error => tracing::error!(target: "wasm_tool", "{}", log.message),
                    }
                }

                // Parse result JSON
                let result: serde_json::Value = serde_json::from_str(&result_json)
                    .unwrap_or(serde_json::Value::String(result_json));

                Ok(ToolOutput::success(result, duration))
            }
            Ok(Err(wasm_err)) => Err(wasm_err.into()),
            Err(_) => Err(WasmError::Timeout(timeout).into()),
        }
    }

    fn requires_sanitization(&self) -> bool {
        // WASM tools always require sanitization - they're untrusted by definition
        true
    }

    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        // Use the timeout as a conservative estimate
        Some(self.prepared.limits.timeout)
    }
}

impl std::fmt::Debug for WasmToolWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmToolWrapper")
            .field("name", &self.prepared.name)
            .field("description", &self.description)
            .field("limits", &self.prepared.limits)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::tools::wasm::capabilities::Capabilities;
    use crate::tools::wasm::runtime::{WasmRuntimeConfig, WasmToolRuntime};

    #[test]
    fn test_wrapper_creation() {
        // This test verifies the runtime can be created
        // Actual execution tests require a valid WASM component
        let config = WasmRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmToolRuntime::new(config).unwrap());

        // Runtime was created successfully
        assert!(runtime.config().fuel_config.enabled);
    }

    #[test]
    fn test_capabilities_default() {
        let caps = Capabilities::default();
        assert!(caps.workspace_read.is_none());
        assert!(caps.http.is_none());
        assert!(caps.tool_invoke.is_none());
        assert!(caps.secrets.is_none());
    }
}
