use std::sync::LazyLock;
use std::time::Instant;

use ironclaw_host_api::result_meta::MODEL_DIAGNOSTIC_MAX_BYTES;
use ironclaw_safety::LeakDetector;
use serde_json::{Map, Value};
use wasmtime::component::Linker;
use wasmtime::{Config, Engine, Store};

use crate::bindings;
use crate::config::{EPOCH_TICK_INTERVAL, WIT_TOOL_VERSION, WitToolRuntimeConfig};
use crate::error::WasmError;
use crate::host::WitToolHost;
use crate::store::StoreData;
use crate::types::{PreparedWitTool, WitToolExecution, WitToolRequest};
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
/// all WASM tools, not just one.
static GUEST_ERROR_LEAK_DETECTOR: LazyLock<LeakDetector> = LazyLock::new(LeakDetector::new);

/// Redact secret-shaped values from a guest-authored error string before it
/// becomes host-visible (`WitToolExecution::error`). Downstream seams (the
/// model-visible diagnostic seam in `ironclaw_loop_host`) still apply their
/// own scrubbing and injection fencing; this is the earlier, sandbox-exit
/// boundary and redacts secret values before bounding the result to the
/// canonical model-diagnostic byte budget.
fn scrub_guest_error(error: String) -> String {
    let (mut scrubbed, _redacted) = GUEST_ERROR_LEAK_DETECTOR.redact_all_secrets(&error);
    if scrubbed.len() <= MODEL_DIAGNOSTIC_MAX_BYTES {
        return scrubbed;
    }

    bound_structured_guest_error(&scrubbed).unwrap_or_else(|| {
        truncate_guest_error(&mut scrubbed);
        scrubbed
    })
}

fn truncate_guest_error(error: &mut String) {
    let mut end = MODEL_DIAGNOSTIC_MAX_BYTES;
    while end > 0 && !error.is_char_boundary(end) {
        end -= 1;
    }
    error.truncate(end);
}

/// Keep the fields understood by the host runtime when a structured guest
/// error is too large. Truncating serialized JSON as plain text can cut a
/// quoted value or the closing object, making the envelope unavailable to the
/// structured-error classifier. Re-serializing a small allowlist preserves
/// the error kind/code while bounding the free-text message.
fn bound_structured_guest_error(error: &str) -> Option<String> {
    let Value::Object(payload) = serde_json::from_str(error).ok()? else {
        return None;
    };
    let kind = payload.get("kind").and_then(Value::as_str)?;
    if kind.is_empty() {
        return None;
    }

    let mut bounded = Map::new();
    bounded.insert("kind".to_string(), Value::String(kind.to_string()));
    let original_code = payload.get("code").and_then(Value::as_str);
    let original_message = payload.get("message").and_then(Value::as_str);
    for field in ["code", "message"] {
        if payload.get(field).and_then(Value::as_str).is_some() {
            bounded.insert(field.to_string(), Value::String(String::new()));
        }
    }

    // Keep the real discriminator and the same optional field shape while
    // fitting. If even that envelope cannot fit, preserving the kind without
    // fabricating a replacement is impossible, so use the text fallback.
    if !serialized_guest_error_fits(&bounded) {
        return None;
    }

    if let Some(code) = original_code {
        bounded.insert("code".to_string(), Value::String(code.to_string()));
    }
    if let Some(message) = original_message {
        bounded.insert("message".to_string(), Value::String(message.to_string()));
    }
    if serialized_guest_error_fits(&bounded) {
        return serde_json::to_string(&bounded).ok();
    }

    // Prefer preserving the complete code when it fits with an empty
    // message. The message then uses only the remaining budget.
    if original_message.is_some() {
        bounded.insert("message".to_string(), Value::String(String::new()));
    }
    if let Some(code) = original_code {
        bounded.insert("code".to_string(), Value::String(code.to_string()));
    }
    if serialized_guest_error_fits(&bounded) {
        if let Some(message) = original_message {
            fit_guest_error_field(&mut bounded, "message", message);
        }
        return serde_json::to_string(&bounded).ok();
    }

    // Otherwise preserve the complete message when it fits with an empty
    // code. The code is then shortened into the remaining budget.
    if let Some(message) = original_message {
        if original_code.is_some() {
            bounded.insert("code".to_string(), Value::String(String::new()));
        }
        bounded.insert("message".to_string(), Value::String(message.to_string()));
        if serialized_guest_error_fits(&bounded) {
            if let Some(code) = original_code {
                fit_guest_error_field(&mut bounded, "code", code);
            }
            return serde_json::to_string(&bounded).ok();
        }
    }

    // Neither optional value fits in full. Keep the real kind, make the
    // declared optional fields empty, and deterministically fit the code. A
    // pathological kind still uses the legacy UTF-8-safe text fallback.
    if original_code.is_some() {
        bounded.insert("code".to_string(), Value::String(String::new()));
    }
    if original_message.is_some() {
        bounded.insert("message".to_string(), Value::String(String::new()));
    }
    if let Some(code) = original_code {
        fit_guest_error_field(&mut bounded, "code", code);
    }
    serde_json::to_string(&bounded).ok()
}

fn serialized_guest_error_fits(payload: &Map<String, Value>) -> bool {
    serde_json::to_vec(payload)
        .is_ok_and(|serialized| serialized.len() <= MODEL_DIAGNOSTIC_MAX_BYTES)
}

/// Trim one JSON string value to the largest UTF-8 prefix that keeps the
/// complete re-serialized envelope within the diagnostic budget.
fn fit_guest_error_field(payload: &mut Map<String, Value>, field: &str, original: &str) {
    debug_assert!(serialized_guest_error_fits(payload));
    let mut low = 0;
    let mut high = original.len();
    while low < high {
        let midpoint = low + (high - low).div_ceil(2);
        let end = utf8_prefix_end(original, midpoint);
        payload.insert(field.to_string(), Value::String(original[..end].to_owned()));
        if serialized_guest_error_fits(payload) {
            low = midpoint;
        } else {
            high = midpoint - 1;
        }
    }

    let end = utf8_prefix_end(original, low);
    payload.insert(field.to_string(), Value::String(original[..end].to_owned()));
    debug_assert!(serialized_guest_error_fits(payload));
}

fn utf8_prefix_end(value: &str, requested: usize) -> usize {
    let mut end = requested.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    end
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
                    scrub_guest_error(error.to_string())
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
            error: response.error.map(scrub_guest_error),
            usage,
            logs,
        })
    }

    fn extract_metadata(
        &self,
        component: &wasmtime::component::Component,
        limits: &SandboxLimits,
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
        limits: &SandboxLimits,
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

    #[test]
    fn scrub_guest_error_bounds_both_structured_optional_fields() {
        let guest_error = serde_json::json!({
            "code": "c".repeat(MODEL_DIAGNOSTIC_MAX_BYTES * 2),
            "kind": "operation_failed",
            "message": "m".repeat(MODEL_DIAGNOSTIC_MAX_BYTES * 2),
            "unknown": "discarded guest metadata",
        })
        .to_string();

        let scrubbed = scrub_guest_error(guest_error);
        let payload: Value = serde_json::from_str(&scrubbed).unwrap();

        assert!(scrubbed.len() <= MODEL_DIAGNOSTIC_MAX_BYTES);
        assert_eq!(payload["kind"], "operation_failed");
        assert!(!payload["code"].as_str().unwrap().is_empty());
        assert_eq!(payload["message"], "");
        assert!(payload.get("unknown").is_none());
    }

    #[test]
    fn scrub_guest_error_bounds_message_without_code() {
        let guest_error = serde_json::json!({
            "kind": "operation_failed",
            "message": "m".repeat(MODEL_DIAGNOSTIC_MAX_BYTES * 2),
        })
        .to_string();

        let scrubbed = scrub_guest_error(guest_error);
        let payload: Value = serde_json::from_str(&scrubbed).unwrap();

        assert!(scrubbed.len() <= MODEL_DIAGNOSTIC_MAX_BYTES);
        assert_eq!(payload["kind"], "operation_failed");
        assert!(!payload["message"].as_str().unwrap().is_empty());
        assert!(payload.get("code").is_none());
    }

    #[test]
    fn scrub_guest_error_uses_text_fallback_for_oversized_kind() {
        let guest_error = serde_json::json!({
            "code": "provider_rejected",
            "kind": "k".repeat(MODEL_DIAGNOSTIC_MAX_BYTES * 2),
            "message": "provider rejected request",
        })
        .to_string();

        let scrubbed = scrub_guest_error(guest_error);

        assert!(scrubbed.len() <= MODEL_DIAGNOSTIC_MAX_BYTES);
        assert!(serde_json::from_str::<Value>(&scrubbed).is_err());
    }
}
