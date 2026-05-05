use std::time::Instant;

use wasmtime::component::Linker;
use wasmtime::{Config, Engine, Store};

use crate::bindings;
use crate::config::{EPOCH_TICK_INTERVAL, WIT_TOOL_VERSION, WitToolLimits, WitToolRuntimeConfig};
use crate::error::WasmError;
use crate::host::WitToolHost;
use crate::store::StoreData;
use crate::types::{PreparedWitTool, WitToolExecution, WitToolRequest};

/// Reborn WIT-compatible WASM tool runtime.
pub struct WitToolRuntime {
    engine: Engine,
    config: WitToolRuntimeConfig,
}

impl WitToolRuntime {
    pub fn new(config: WitToolRuntimeConfig) -> Result<Self, WasmError> {
        let mut wasmtime_config = Config::new();
        wasmtime_config.wasm_component_model(true);
        wasmtime_config.wasm_threads(false);
        wasmtime_config.consume_fuel(true);
        wasmtime_config.epoch_interruption(true);
        wasmtime_config.debug_info(false);

        let engine = Engine::new(&wasmtime_config)
            .map_err(|error| WasmError::EngineCreationFailed(error.to_string()))?;
        spawn_epoch_ticker(engine.clone())?;

        Ok(Self { engine, config })
    }

    pub fn config(&self) -> &WitToolRuntimeConfig {
        &self.config
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn prepare(&self, name: &str, wasm_bytes: &[u8]) -> Result<PreparedWitTool, WasmError> {
        let component = wasmtime::component::Component::new(&self.engine, wasm_bytes)
            .map_err(|error| WasmError::CompilationFailed(error.to_string()))?;
        let limits = self.config.default_limits.clone();
        let (description, schema) = self.extract_metadata(&component, &limits)?;

        Ok(PreparedWitTool {
            name: name.to_string(),
            description,
            schema,
            component,
            limits,
        })
    }

    pub fn execute(
        &self,
        prepared: &PreparedWitTool,
        host: WitToolHost,
        request: WitToolRequest,
    ) -> Result<WitToolExecution, WasmError> {
        let started = Instant::now();
        let (mut store, instance) =
            self.instantiate(&prepared.component, host, &prepared.limits)?;
        let tool = instance.near_agent_tool();
        let request = bindings::exports::near::agent::tool::Request {
            params: request.params_json,
            context: request.context_json,
        };
        let response = match tool.call_execute(&mut store, &request) {
            Ok(response) => response,
            Err(error) => {
                let message = if store.data().deadline_exceeded() {
                    "WASM execution deadline exceeded".to_string()
                } else {
                    error.to_string()
                };
                return Err(execution_failed_with_usage(message, &store, started));
            }
        };
        if store.data().deadline_exceeded() {
            return Err(execution_failed_with_usage(
                "WASM execution deadline exceeded".to_string(),
                &store,
                started,
            ));
        }

        let mut usage = store.data().usage.clone();
        usage.wall_clock_ms = elapsed_millis(started);
        usage.output_bytes = response
            .output
            .as_deref()
            .map(|output| output.len().min(u64::MAX as usize) as u64)
            .unwrap_or(0);
        let logs = store.data().logs.clone();

        Ok(WitToolExecution {
            output_json: response.output,
            error: response.error,
            usage,
            logs,
        })
    }

    fn extract_metadata(
        &self,
        component: &wasmtime::component::Component,
        limits: &WitToolLimits,
    ) -> Result<(String, serde_json::Value), WasmError> {
        let (mut store, instance) = self.instantiate(component, WitToolHost::deny_all(), limits)?;
        let tool = instance.near_agent_tool();
        let description = tool
            .call_description(&mut store)
            .map_err(|error| WasmError::execution_failed(error.to_string()))?;
        let schema_json = tool
            .call_schema(&mut store)
            .map_err(|error| WasmError::execution_failed(error.to_string()))?;
        let schema = serde_json::from_str::<serde_json::Value>(&schema_json)
            .map_err(|error| WasmError::InvalidSchema(error.to_string()))?;
        if !schema.is_object() {
            return Err(WasmError::InvalidSchema(
                "schema export must return a JSON object".to_string(),
            ));
        }
        Ok((description, schema))
    }

    fn instantiate(
        &self,
        component: &wasmtime::component::Component,
        host: WitToolHost,
        limits: &WitToolLimits,
    ) -> Result<(Store<StoreData>, bindings::SandboxedTool), WasmError> {
        let mut store = Store::new(
            &self.engine,
            StoreData::new(host, limits.memory_bytes, limits.timeout),
        );
        configure_store(&mut store, limits)?;
        let linker = create_linker(&self.engine)?;
        let instance = bindings::SandboxedTool::instantiate(&mut store, component, &linker)
            .map_err(|error| classify_instantiation_error(error.to_string()))?;
        Ok((store, instance))
    }
}

impl std::fmt::Debug for WitToolRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WitToolRuntime")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

fn spawn_epoch_ticker(engine: Engine) -> Result<(), WasmError> {
    std::thread::Builder::new()
        .name("reborn-wasm-epoch-ticker".into())
        .spawn(move || {
            loop {
                std::thread::sleep(EPOCH_TICK_INTERVAL);
                engine.increment_epoch();
            }
        })
        .map(|_| ())
        .map_err(|error| WasmError::EngineCreationFailed(error.to_string()))
}

fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn execution_failed_with_usage(
    message: String,
    store: &Store<StoreData>,
    started: Instant,
) -> WasmError {
    let mut usage = store.data().usage.clone();
    usage.wall_clock_ms = elapsed_millis(started);
    WasmError::ExecutionFailed {
        message,
        usage,
        logs: store.data().logs.clone(),
    }
}

fn configure_store(store: &mut Store<StoreData>, limits: &WitToolLimits) -> Result<(), WasmError> {
    store
        .set_fuel(limits.fuel)
        .map_err(|error| WasmError::StoreConfiguration(error.to_string()))?;
    store.epoch_deadline_trap();
    let ticks = (limits.timeout.as_millis() / EPOCH_TICK_INTERVAL.as_millis()).max(1) as u64;
    store.set_epoch_deadline(ticks);
    store.limiter(|data| &mut data.limiter);
    Ok(())
}

fn create_linker(engine: &Engine) -> Result<Linker<StoreData>, WasmError> {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
        .map_err(|error| WasmError::LinkerConfiguration(error.to_string()))?;
    bindings::SandboxedTool::add_to_linker::<_, wasmtime::component::HasSelf<_>>(
        &mut linker,
        |state: &mut StoreData| state,
    )
    .map_err(|error| WasmError::LinkerConfiguration(error.to_string()))?;
    Ok(linker)
}

fn classify_instantiation_error(message: String) -> WasmError {
    if message.contains("near:agent") || message.contains("import") {
        WasmError::InstantiationFailed(format!(
            "{message}. This usually means the component was compiled against a different WIT version than the host supports (host: {WIT_TOOL_VERSION})."
        ))
    } else {
        WasmError::InstantiationFailed(message)
    }
}
