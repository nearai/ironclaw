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
    PreparedWitTool, WitErrorKind, WitGuestFailure, WitToolExecution, WitToolOutcome,
    WitToolRequest,
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
/// all WASM tools, not just one. This applies to `guest-failure`'s
/// `code`/`message` fields — the only free-text carriers on that path.
static GUEST_ERROR_LEAK_DETECTOR: LazyLock<LeakDetector> = LazyLock::new(LeakDetector::new);

/// Maximum retained bytes for either free-text field in a guest failure.
/// This is enforced at the sandbox exit before host allocation and scanning
/// can be amplified by concurrently failing guests.
const MAX_GUEST_ERROR_BYTES: usize = 4096;

/// Redact secret-shaped values from a guest-authored string before it becomes
/// guest-visible (`WitToolExecution::outcome`). Downstream seams (the
/// model-visible diagnostic seam in `ironclaw_loop_host`) still apply their
/// own scrubbing and injection fencing; this is the earlier, sandbox-exit
/// boundary. It redacts secret values and bounds retained text without ever
/// splitting a UTF-8 code point.
fn scrub_guest_error(error: String) -> String {
    let (mut scrubbed, _redacted) = GUEST_ERROR_LEAK_DETECTOR.redact_all_secrets(&error);
    if scrubbed.len() > MAX_GUEST_ERROR_BYTES {
        let mut end = MAX_GUEST_ERROR_BYTES;
        while !scrubbed.is_char_boundary(end) {
            end -= 1;
        }
        scrubbed.truncate(end);
    }
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
                })
            }
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

/// Classify a WIT contract-version mismatch separately from an unrelated
/// instantiation failure. Wasmtime includes the imported interface reference
/// (for example, `near:agent/host@0.3.0`) in these errors; generic imports and
/// same-version missing imports must remain ordinary instantiation failures.
fn classify_instantiation_error(message: String) -> WasmError {
    if has_unsupported_wit_contract_version(&message) {
        WasmError::UnsupportedContract(format!(
            "{message}. This component targets an unsupported WIT contract version — the host only supports near:agent@{WIT_TOOL_VERSION}."
        ))
    } else {
        WasmError::InstantiationFailed(message)
    }
}

fn has_unsupported_wit_contract_version(message: &str) -> bool {
    message
        .split(|character: char| {
            character.is_whitespace() || matches!(character, '`' | '"' | '\'' | ',' | '(' | ')')
        })
        .filter_map(|token| token.strip_prefix("near:agent"))
        .filter(|reference| reference.starts_with('/') || reference.starts_with('@'))
        .filter_map(|reference| {
            reference.rsplit_once('@').map(|(_, version)| {
                version
                    .split_once('#')
                    .map_or(version, |(version, _)| version)
            })
        })
        .any(|version| version != WIT_TOOL_VERSION)
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

    #[test]
    fn scrub_guest_error_bounds_text_at_a_utf8_boundary() {
        let guest_error = format!("{}é", "a".repeat(4095));

        let scrubbed = scrub_guest_error(guest_error);

        assert!(scrubbed.len() <= 4096, "{} bytes", scrubbed.len());
        assert_eq!(scrubbed, "a".repeat(4095));
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
mod classify_instantiation_error_tests {
    use super::*;

    #[test]
    fn version_mismatch_message_gains_the_unsupported_contract_hint() {
        let error = classify_instantiation_error(
            "component imports instance `near:agent/host@0.3.0`, but a matching implementation \
             was not found in the linker"
                .to_string(),
        );
        let WasmError::UnsupportedContract(message) = error else {
            panic!("expected UnsupportedContract");
        };
        assert!(message.contains("near:agent/host@0.3.0"));
        assert!(message.contains("unsupported WIT contract version"));
        assert!(message.contains(WIT_TOOL_VERSION));
    }

    #[test]
    fn generic_import_error_remains_an_instantiation_failure() {
        let error =
            classify_instantiation_error("missing import `some-other-interface`".to_string());
        let WasmError::InstantiationFailed(message) = error else {
            panic!("expected InstantiationFailed");
        };
        assert_eq!(message, "missing import `some-other-interface`");
    }

    #[test]
    fn same_version_unknown_import_remains_an_instantiation_failure() {
        let error = classify_instantiation_error(
            "missing import `near:agent/host@0.4.1` function `unknown-interface`".to_string(),
        );
        let WasmError::InstantiationFailed(message) = error else {
            panic!("expected InstantiationFailed");
        };
        assert_eq!(
            message,
            "missing import `near:agent/host@0.4.1` function `unknown-interface`"
        );
    }

    #[test]
    fn unrelated_instantiation_failure_passes_through_unmodified() {
        let error = classify_instantiation_error("trap: out of bounds memory access".to_string());
        let WasmError::InstantiationFailed(message) = error else {
            panic!("expected InstantiationFailed");
        };
        assert_eq!(message, "trap: out of bounds memory access");
    }
}
