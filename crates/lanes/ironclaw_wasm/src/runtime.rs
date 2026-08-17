use std::sync::LazyLock;
use std::time::Instant;

use ironclaw_safety::LeakDetector;
use wasmtime::component::Linker;
use wasmtime::{Config, Engine, Store};

use crate::bindings;
use crate::config::{EPOCH_TICK_INTERVAL, WIT_TOOL_VERSION, WitToolRuntimeConfig};
use crate::error::WasmError;
use crate::host::WitToolHost;
use crate::store::StoreData;
use crate::types::{
    PreparedWitTool, WitBindingVersion, WitErrorKind, WitGuestFailure, WitToolExecution,
    WitToolOutcome, WitToolRequest,
};
use crate::wasm_sandbox_core::SandboxLimits;

/// Shared leak-detector registry (well-known vendor API-token shapes,
/// PEM/SSH keys, bearer/JWT, …) used to scrub guest-authored error text
/// before it crosses the WASM sandbox boundary into host-controlled data.
///
/// Guests are sandboxed but not trusted with free text on the error channel:
/// a provider HTTP body echoed verbatim by a guest (e.g. a rejected-request
/// response body that repeats the credential the caller sent) can carry live
/// credential material. This is the single chokepoint every WIT tool's guest
/// error crosses on the way out of the sandbox, so redacting here defends
/// all WASM tools, not just one. For the typed (current) response, this
/// applies to `guest-failure`'s `code`/`message` fields — the only free-text
/// carriers on that path; for the legacy response, it applies to the whole
/// error string, as before.
static GUEST_ERROR_LEAK_DETECTOR: LazyLock<LeakDetector> = LazyLock::new(LeakDetector::new);

/// Redact secret-shaped values from a guest-authored string before it becomes
/// guest-visible (`WitToolExecution::outcome`). Downstream seams (the
/// model-visible diagnostic seam in `ironclaw_loop_host`) still apply their
/// own scrubbing and injection fencing; this is the earlier, sandbox-exit
/// boundary and only redacts secret values in place — it never blocks or
/// truncates the string, so the descriptive cause survives.
fn scrub_guest_error(error: String) -> String {
    let (scrubbed, _redacted) = GUEST_ERROR_LEAK_DETECTOR.redact_all_secrets(&error);
    scrubbed
}

fn scrub_guest_error_opt(error: Option<String>) -> Option<String> {
    error.map(scrub_guest_error)
}

/// Reborn WIT-compatible WASM tool runtime.
///
/// Cloning is cheap: [`Engine`] is internally reference-counted and
/// [`WitToolRuntimeConfig`] is a small `Clone` value. A clone shares the same
/// underlying wasmtime engine, so a clone can be moved into a blocking task
/// (`tokio::task::spawn_blocking`) to run the synchronous guest call off the
/// async worker pool without re-creating the engine.
#[derive(Clone)]
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
        let (description, schema, binding_version) = self.extract_metadata(&component, &limits)?;

        Ok(PreparedWitTool {
            name: name.to_string(),
            description,
            schema,
            component,
            limits,
            binding_version,
        })
    }

    pub fn execute(
        &self,
        prepared: &PreparedWitTool,
        host: WitToolHost,
        request: WitToolRequest,
    ) -> Result<WitToolExecution, WasmError> {
        match prepared.binding_version {
            WitBindingVersion::Current => self.execute_current(prepared, host, request),
            // Legacy fallback — removed in PR 4.
            WitBindingVersion::Legacy => self.execute_legacy(prepared, host, request),
        }
    }

    fn execute_current(
        &self,
        prepared: &PreparedWitTool,
        host: WitToolHost,
        request: WitToolRequest,
    ) -> Result<WitToolExecution, WasmError> {
        let started = Instant::now();
        let (mut store, instance) =
            self.instantiate_current(&prepared.component, host, &prepared.limits)?;
        let tool = instance.near_agent_tool();
        let wit_request = bindings::exports::near::agent::tool::Request {
            params: request.params_json,
            context: request.context_json,
        };
        let response = match tool.call_execute(&mut store, &wit_request) {
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

        let output_bytes = match &response {
            bindings::exports::near::agent::tool::Response::Success(output) => {
                output.len().min(u64::MAX as usize) as u64
            }
            bindings::exports::near::agent::tool::Response::Failure(_) => 0,
        };
        let mut usage = store.data().usage.clone();
        usage.wall_clock_ms = elapsed_millis(started);
        usage.output_bytes = output_bytes;
        let logs = store.data().logs.clone();

        let outcome = match response {
            bindings::exports::near::agent::tool::Response::Success(output) => {
                WitToolOutcome::Success(output)
            }
            bindings::exports::near::agent::tool::Response::Failure(failure) => {
                WitToolOutcome::Failure(WitGuestFailure {
                    kind: map_error_kind(failure.kind),
                    code: scrub_guest_error_opt(failure.code),
                    message: scrub_guest_error_opt(failure.message),
                    retry_after_ms: failure.retry_after_ms,
                })
            }
        };

        Ok(WitToolExecution {
            outcome,
            usage,
            logs,
        })
    }

    /// Legacy 0.3.0 binding fallback — removed in PR 4.
    fn execute_legacy(
        &self,
        prepared: &PreparedWitTool,
        host: WitToolHost,
        request: WitToolRequest,
    ) -> Result<WitToolExecution, WasmError> {
        let started = Instant::now();
        let (mut store, instance) =
            self.instantiate_legacy(&prepared.component, host, &prepared.limits)?;
        let tool = instance.near_agent_tool();
        let wit_request = bindings::legacy::exports::near::agent::tool::Request {
            params: request.params_json,
            context: request.context_json,
        };
        let response = match tool.call_execute(&mut store, &wit_request) {
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

        let outcome = match (response.output, response.error) {
            (_, Some(error)) => WitToolOutcome::LegacyFailure(scrub_guest_error(error)),
            (Some(output), None) => WitToolOutcome::Success(output),
            (None, None) => WitToolOutcome::LegacyMissingOutput,
        };

        Ok(WitToolExecution {
            outcome,
            usage,
            logs,
        })
    }

    fn extract_metadata(
        &self,
        component: &wasmtime::component::Component,
        limits: &SandboxLimits,
    ) -> Result<(String, serde_json::Value, WitBindingVersion), WasmError> {
        match self.instantiate_current(component, WitToolHost::deny_all(), limits) {
            Ok((mut store, instance)) => {
                let tool = instance.near_agent_tool();
                let description = tool
                    .call_description(&mut store)
                    .map_err(|error| WasmError::execution_failed(error.to_string()))?;
                let schema_json = tool
                    .call_schema(&mut store)
                    .map_err(|error| WasmError::execution_failed(error.to_string()))?;
                let schema = parse_schema(&schema_json)?;
                Ok((description, schema, WitBindingVersion::Current))
            }
            // Legacy fallback — removed in PR 4. A component compiled against
            // the frozen 0.3.0 world fails the current-world instantiation on
            // an import/version mismatch (wasmtime's instantiation error
            // names the missing import, e.g. `near:agent/host`); only that
            // signature is worth retrying against the legacy world. Any
            // other current-world failure (a real bug in the component, a
            // resource limit, ...) is not a version issue and re-running it
            // against a different world would just mask the real error.
            Err(current_error) if is_version_mismatch_error(&current_error) => {
                match self.instantiate_legacy(component, WitToolHost::deny_all(), limits) {
                    Ok((mut store, instance)) => {
                        let tool = instance.near_agent_tool();
                        let description = tool
                            .call_description(&mut store)
                            .map_err(|error| WasmError::execution_failed(error.to_string()))?;
                        let schema_json = tool
                            .call_schema(&mut store)
                            .map_err(|error| WasmError::execution_failed(error.to_string()))?;
                        let schema = parse_schema(&schema_json)?;
                        Ok((description, schema, WitBindingVersion::Legacy))
                    }
                    Err(legacy_error) => Err(WasmError::InstantiationFailed(format!(
                        "current-world instantiation failed: {current_error}; legacy-world fallback also failed: {legacy_error}"
                    ))),
                }
            }
            Err(current_error) => Err(current_error),
        }
    }

    fn instantiate_current(
        &self,
        component: &wasmtime::component::Component,
        host: WitToolHost,
        limits: &SandboxLimits,
    ) -> Result<(Store<StoreData>, bindings::SandboxedTool), WasmError> {
        let mut store = Store::new(
            &self.engine,
            StoreData::new(host, limits.memory_bytes, limits.timeout),
        );
        configure_store(&mut store, limits)?;
        let linker = create_linker_current(&self.engine)?;
        let instance = bindings::SandboxedTool::instantiate(&mut store, component, &linker)
            .map_err(|error| classify_instantiation_error(error.to_string()))?;
        Ok((store, instance))
    }

    /// Legacy 0.3.0 binding fallback — removed in PR 4.
    fn instantiate_legacy(
        &self,
        component: &wasmtime::component::Component,
        host: WitToolHost,
        limits: &SandboxLimits,
    ) -> Result<(Store<StoreData>, bindings::legacy::SandboxedTool), WasmError> {
        let mut store = Store::new(
            &self.engine,
            StoreData::new(host, limits.memory_bytes, limits.timeout),
        );
        configure_store(&mut store, limits)?;
        let linker = create_linker_legacy(&self.engine)?;
        let instance = bindings::legacy::SandboxedTool::instantiate(&mut store, component, &linker)
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

fn parse_schema(schema_json: &str) -> Result<serde_json::Value, WasmError> {
    let schema = serde_json::from_str::<serde_json::Value>(schema_json)
        .map_err(|error| WasmError::InvalidSchema(error.to_string()))?;
    if !schema.is_object() {
        return Err(WasmError::InvalidSchema(
            "schema export must return a JSON object".to_string(),
        ));
    }
    Ok(schema)
}

fn map_error_kind(kind: bindings::exports::near::agent::tool::ErrorKind) -> WitErrorKind {
    use bindings::exports::near::agent::tool::ErrorKind as WitKind;
    match kind {
        WitKind::AuthRequired => WitErrorKind::AuthRequired,
        WitKind::Input => WitErrorKind::Input,
        WitKind::OutputTooLarge => WitErrorKind::OutputTooLarge,
        WitKind::Executor => WitErrorKind::Executor,
        WitKind::NetworkDenied => WitErrorKind::NetworkDenied,
        WitKind::Client => WitErrorKind::Client,
        WitKind::OperationFailed => WitErrorKind::OperationFailed,
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

fn configure_store(store: &mut Store<StoreData>, limits: &SandboxLimits) -> Result<(), WasmError> {
    store
        .set_fuel(limits.fuel)
        .map_err(|error| WasmError::StoreConfiguration(error.to_string()))?;
    store.epoch_deadline_trap();
    let ticks = (limits.timeout.as_millis() / EPOCH_TICK_INTERVAL.as_millis()).max(1) as u64;
    store.set_epoch_deadline(ticks);
    store.limiter(|data| &mut data.limiter);
    Ok(())
}

fn create_linker_current(engine: &Engine) -> Result<Linker<StoreData>, WasmError> {
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

/// Legacy 0.3.0 binding fallback — removed in PR 4.
fn create_linker_legacy(engine: &Engine) -> Result<Linker<StoreData>, WasmError> {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
        .map_err(|error| WasmError::LinkerConfiguration(error.to_string()))?;
    bindings::legacy::SandboxedTool::add_to_linker::<_, wasmtime::component::HasSelf<_>>(
        &mut linker,
        |state: &mut StoreData| state,
    )
    .map_err(|error| WasmError::LinkerConfiguration(error.to_string()))?;
    Ok(linker)
}

/// Whether `error` looks like a current-world instantiation failure caused by
/// a version/import mismatch rather than a real component bug. Wasmtime
/// names the missing import in its instantiation error text (e.g.
/// `near:agent/host@0.4.0`), so a plain substring check on `near:agent` is
/// enough to distinguish "this component targets a different WIT version"
/// from "this component is broken" without a new abstraction.
fn is_version_mismatch_error(error: &WasmError) -> bool {
    matches!(error, WasmError::InstantiationFailed(message) if message.contains("near:agent"))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Recognized by `LeakDetector`'s `github_token` pattern
    /// (`ironclaw_safety::leak_detector::test_detect_github_token`).
    const GITHUB_TOKEN_SHAPE: &str = "ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

    #[test]
    fn scrub_guest_error_redacts_leak_detector_recognized_secret_shapes() {
        let guest_error = format!("upstream rejected request: token {GITHUB_TOKEN_SHAPE} failed");

        let scrubbed = scrub_guest_error(guest_error.clone());

        assert_ne!(scrubbed, guest_error);
        assert!(!scrubbed.contains(GITHUB_TOKEN_SHAPE));
    }

    #[test]
    fn scrub_guest_error_leaves_benign_text_unchanged() {
        let guest_error = "channel_not_found: no such Slack channel".to_string();

        let scrubbed = scrub_guest_error(guest_error.clone());

        assert_eq!(scrubbed, guest_error);
    }

    /// Integration-ish: pins the exact seam `WitToolRuntime::execute` uses —
    /// `response.error.map(scrub_guest_error)` — so a guest-authored error
    /// carrying a secret-shaped value is redacted before it can populate
    /// `WitToolExecution::error`, the value that crosses the sandbox
    /// boundary into host-controlled data.
    #[test]
    fn execute_seam_scrubs_guest_error_before_reaching_wit_tool_execution() {
        let guest_error = Some(format!("leaked cred: {GITHUB_TOKEN_SHAPE}"));

        let scrubbed_error: Option<String> = guest_error.map(scrub_guest_error);

        assert!(!scrubbed_error.unwrap().contains(GITHUB_TOKEN_SHAPE));
    }
}

#[cfg(test)]
mod legacy_retry_gate_tests {
    use super::*;

    /// A current-world instantiation failure that names the missing
    /// `near:agent` import is the version-mismatch signature `extract_metadata`
    /// gates its legacy-world retry on.
    #[test]
    fn version_mismatch_signature_is_detected_from_instantiation_error() {
        let error = classify_instantiation_error(
            "unknown import: `near:agent/host@0.4.0::log` has not been defined".to_string(),
        );
        assert!(is_version_mismatch_error(&error));
    }

    /// A current-world instantiation failure that does NOT name a
    /// `near:agent` import (a real component bug, not a version mismatch)
    /// must not be treated as retry-worthy — retrying it against the legacy
    /// world would mask the real error instead of surfacing it.
    #[test]
    fn non_version_instantiation_error_is_not_a_retry_signature() {
        let error = classify_instantiation_error("trap: out of bounds memory access".to_string());
        assert!(!is_version_mismatch_error(&error));
    }

    /// Non-instantiation `WasmError` variants (store/linker configuration
    /// failures preceding instantiation) are never retry-worthy either —
    /// the gate only matches `InstantiationFailed`.
    #[test]
    fn non_instantiation_error_variant_is_not_a_retry_signature() {
        let error =
            WasmError::StoreConfiguration("near:agent memory limit misconfigured".to_string());
        assert!(!is_version_mismatch_error(&error));
    }
}
