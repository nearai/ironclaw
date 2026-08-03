use ironclaw_host_api::ids::CapabilityId;
use ironclaw_wasm::{WasmError, WasmLogLevel, WasmLogRecord, sanitize_wasm_diagnostic};

pub(super) fn log_wasm_runtime_error(capability_id: &CapabilityId, error: &WasmError) {
    if let WasmError::ExecutionFailed { message, logs, .. } = error {
        log_wasm_guest_logs(capability_id, logs);
        let message = sanitize_wasm_diagnostic(message);
        tracing::debug!(
            capability_id = %capability_id,
            wasm_error = %message,
            "WASM runtime execution failed with guest diagnostic"
        );
        return;
    }

    let error = sanitize_wasm_diagnostic(error.to_string());
    tracing::debug!(
        capability_id = %capability_id,
        wasm_error = %error,
        "WASM runtime execution failed"
    );
}

pub(super) fn log_wasm_guest_error(
    capability_id: &CapabilityId,
    logs: &[WasmLogRecord],
    error: &str,
) {
    log_wasm_guest_logs(capability_id, logs);
    let error = sanitize_wasm_diagnostic(error);
    tracing::debug!(
        capability_id = %capability_id,
        wasm_error = %error,
        "WASM guest returned capability error diagnostic"
    );
}

fn log_wasm_guest_logs(capability_id: &CapabilityId, logs: &[WasmLogRecord]) {
    for log in logs {
        let message = sanitize_wasm_diagnostic(&log.message);
        match log.level {
            WasmLogLevel::Trace => tracing::trace!(
                capability_id = %capability_id,
                wasm_log = %message,
                "WASM guest log"
            ),
            WasmLogLevel::Debug => tracing::debug!(
                capability_id = %capability_id,
                wasm_log = %message,
                "WASM guest log"
            ),
            WasmLogLevel::Info => tracing::info!(
                capability_id = %capability_id,
                wasm_log = %message,
                "WASM guest log"
            ),
            WasmLogLevel::Warn => tracing::warn!(
                capability_id = %capability_id,
                wasm_log = %message,
                "WASM guest log"
            ),
            WasmLogLevel::Error => tracing::error!(
                capability_id = %capability_id,
                wasm_log = %message,
                "WASM guest log"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::sync::{Arc, Mutex};

    use ironclaw_extensions::{
        CapabilityProviderHostApiContract, ExtensionManifest, ExtensionPackage,
        HostApiContractRegistry, ManifestSource,
    };
    use ironclaw_filesystem::DiskFilesystem;
    use ironclaw_host_api::{
        capability::{CapabilityDescriptor, PermissionMode},
        host_port::HostPortCatalog,
        ids::{CapabilityId, ExtensionId},
        path::VirtualPath,
        resource::{ResourceEstimate, ResourceScope, ResourceUsage},
        runtime::{RuntimeKind, TrustClass},
        runtime_policy::{
            ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy,
            FilesystemBackendKind, NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
        },
    };
    use ironclaw_resources::InMemoryResourceGovernor;
    use ironclaw_wasm::{
        WASM_DIAGNOSTIC_MAX_BYTES, WASM_DIAGNOSTIC_REDACTION_MARKER, WitToolHost, WitToolRuntime,
        WitToolRuntimeConfig,
    };
    use serde_json::json;
    use tracing::field::{Field, Visit};
    use tracing::{Event, Level, Subscriber};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::registry::LookupSpan;
    use tracing_subscriber::{Layer, Registry};
    use wit_component::{ComponentEncoder, StringEncoding, embed_component_metadata};
    use wit_parser::Resolve;

    use super::super::runtime_adapters::RuntimeLaneRequest;
    use super::super::wasm_execution::execute_prepared_wasm;
    use super::*;

    const DETECTABLE_SECRET: &str = "AKIAIOSFODNN7EXAMPLE";
    const DIAGNOSTIC_TARGET: &str = "ironclaw_host_runtime::services::wasm_diagnostics";

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct CapturedEvent {
        level: Level,
        target: String,
        fields: BTreeMap<String, String>,
    }

    #[derive(Clone, Default)]
    struct CapturingLayer {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl<S> Layer<S> for CapturingLayer
    where
        S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
            let mut visitor = FieldVisitor::default();
            event.record(&mut visitor);
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(CapturedEvent {
                    level: *event.metadata().level(),
                    target: event.metadata().target().to_string(),
                    fields: visitor.fields,
                });
        }
    }

    #[derive(Default)]
    struct FieldVisitor {
        fields: BTreeMap<String, String>,
    }

    impl Visit for FieldVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .insert(field.name().to_string(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }
    }

    fn capability_id() -> CapabilityId {
        CapabilityId::new("test.wasm-diagnostics").expect("valid test capability id")
    }

    fn capture_events(action: impl FnOnce()) -> Vec<CapturedEvent> {
        let layer = CapturingLayer::default();
        let events = Arc::clone(&layer.events);
        let subscriber = Registry::default().with(layer);
        tracing::subscriber::with_default(subscriber, action);
        Arc::try_unwrap(events)
            .expect("capture is no longer shared")
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    async fn capture_events_async<F>(future: F) -> (F::Output, Vec<CapturedEvent>)
    where
        F: Future,
    {
        let layer = CapturingLayer::default();
        let events = Arc::clone(&layer.events);
        let subscriber = Registry::default().with(layer);
        let guard = tracing::subscriber::set_default(subscriber);
        let output = future.await;
        drop(guard);
        let events = Arc::try_unwrap(events)
            .expect("capture is no longer shared")
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (output, events)
    }

    fn field<'a>(event: &'a CapturedEvent, name: &str) -> &'a str {
        event
            .fields
            .get(name)
            .map(String::as_str)
            .unwrap_or_else(|| panic!("event is missing {name}: {event:?}"))
    }

    fn diagnostic_fields(events: &[CapturedEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|event| {
                event
                    .fields
                    .get("wasm_log")
                    .or_else(|| event.fields.get("wasm_error"))
                    .map(String::as_str)
            })
            .collect()
    }

    #[test]
    fn host_tracing_sanitizes_every_raw_wasm_diagnostic_source() {
        let capability_id = capability_id();
        let levels = [
            WasmLogLevel::Trace,
            WasmLogLevel::Debug,
            WasmLogLevel::Info,
            WasmLogLevel::Warn,
            WasmLogLevel::Error,
        ];
        let guest_logs = levels
            .into_iter()
            .map(|level| WasmLogRecord {
                level,
                message: format!("guest supplied {DETECTABLE_SECRET}"),
            })
            .collect::<Vec<_>>();
        let execution_error = WasmError::ExecutionFailed {
            message: format!("trap contained {DETECTABLE_SECRET}"),
            usage: ResourceUsage::default(),
            logs: guest_logs,
        };
        let legacy_error =
            WasmError::CompilationFailed(format!("legacy diagnostic {DETECTABLE_SECRET}"));

        let events = capture_events(|| {
            log_wasm_runtime_error(&capability_id, &execution_error);
            log_wasm_guest_error(
                &capability_id,
                &[],
                &format!("guest response {DETECTABLE_SECRET}"),
            );
            log_wasm_runtime_error(&capability_id, &legacy_error);
        });

        assert_eq!(events.len(), 8, "every diagnostic must retain its route");
        assert!(
            events.iter().all(|event| event.target == DIAGNOSTIC_TARGET),
            "sanitization must not reroute WASM diagnostics: {events:?}"
        );
        assert!(
            events
                .iter()
                .all(|event| field(event, "capability_id") == capability_id.as_str()),
            "sanitization must retain capability routing: {events:?}"
        );
        assert_eq!(
            events
                .iter()
                .take(5)
                .map(|event| event.level)
                .collect::<Vec<_>>(),
            vec![
                Level::TRACE,
                Level::DEBUG,
                Level::INFO,
                Level::WARN,
                Level::ERROR,
            ],
            "guest log levels must survive sanitization"
        );

        let diagnostics = diagnostic_fields(&events);
        assert_eq!(diagnostics.len(), events.len());
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.contains(DETECTABLE_SECRET)),
            "a detectable credential reached host tracing: {events:?}"
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.contains(WASM_DIAGNOSTIC_REDACTION_MARKER)),
            "each unsafe cause must be replaced with the stable marker: {events:?}"
        );
    }

    #[test]
    fn host_tracing_preserves_benign_wasm_diagnostics_and_safe_event_wording() {
        let capability_id = capability_id();
        let execution_error = WasmError::ExecutionFailed {
            message: "benign execution detail".to_string(),
            usage: ResourceUsage::default(),
            logs: vec![WasmLogRecord {
                level: WasmLogLevel::Info,
                message: "benign guest log".to_string(),
            }],
        };
        let legacy_error = WasmError::CompilationFailed("benign compiler detail".to_string());

        let events = capture_events(|| {
            log_wasm_runtime_error(&capability_id, &execution_error);
            log_wasm_guest_error(&capability_id, &[], "benign guest response");
            log_wasm_runtime_error(&capability_id, &legacy_error);
        });

        assert_eq!(events.len(), 4);
        assert_eq!(field(&events[0], "wasm_log"), "benign guest log");
        assert_eq!(field(&events[1], "wasm_error"), "benign execution detail");
        assert_eq!(field(&events[2], "wasm_error"), "benign guest response");
        assert_eq!(
            field(&events[3], "wasm_error"),
            "failed to compile WIT component: benign compiler detail"
        );
        assert!(
            events.iter().all(|event| {
                let message = field(event, "message");
                !message.contains("raw guest error") && !message.contains("raw capability error")
            }),
            "event wording must not promise that guest-controlled text is raw: {events:?}"
        );
    }

    #[test]
    fn host_tracing_redaction_marker_is_idempotent_at_the_sink() {
        let capability_id = capability_id();
        let events = capture_events(|| {
            log_wasm_guest_error(&capability_id, &[], WASM_DIAGNOSTIC_REDACTION_MARKER);
        });

        assert_eq!(events.len(), 1);
        assert_eq!(
            field(&events[0], "wasm_error"),
            WASM_DIAGNOSTIC_REDACTION_MARKER,
            "re-scanning an already sanitized diagnostic must not rewrite or nest its marker"
        );
    }

    #[test]
    fn host_tracing_accepts_the_byte_boundary_and_wholly_redacts_oversize_inputs() {
        let capability_id = capability_id();
        let at_limit = "é".repeat(WASM_DIAGNOSTIC_MAX_BYTES / "é".len());
        let over_limit = format!("{at_limit}x");
        assert_eq!(at_limit.len(), WASM_DIAGNOSTIC_MAX_BYTES);
        assert_eq!(over_limit.len(), WASM_DIAGNOSTIC_MAX_BYTES + 1);

        let execution_error = WasmError::ExecutionFailed {
            message: over_limit.clone(),
            usage: ResourceUsage::default(),
            logs: vec![WasmLogRecord {
                level: WasmLogLevel::Info,
                message: at_limit.clone(),
            }],
        };
        let legacy_error = WasmError::CompilationFailed(over_limit.clone());
        let events = capture_events(|| {
            log_wasm_runtime_error(&capability_id, &execution_error);
            log_wasm_guest_error(&capability_id, &[], &over_limit);
            log_wasm_runtime_error(&capability_id, &legacy_error);
        });

        assert_eq!(events.len(), 4);
        assert_eq!(
            field(&events[0], "wasm_log"),
            at_limit,
            "a complete UTF-8 diagnostic at the byte limit remains available"
        );
        for event in &events[1..] {
            let diagnostic = field(event, "wasm_error");
            assert_eq!(
                diagnostic, WASM_DIAGNOSTIC_REDACTION_MARKER,
                "an oversize diagnostic must be replaced wholesale: {event:?}"
            );
            assert!(diagnostic.len() <= WASM_DIAGNOSTIC_MAX_BYTES);
            assert!(
                !diagnostic.contains(&over_limit),
                "no oversize cause fragment may survive: {event:?}"
            );
        }
    }

    const CALLER_TRACE_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "wasm-trace-fixture"
name = "WASM trace fixture"
version = "0.1.0"
description = "Caller-level WASM tracing fixture"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "fixture.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "wasm-trace-fixture.run"
description = "Run tracing fixture"
effects = ["network"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/wasm-trace-fixture/run.input.v1.json"
output_schema_ref = "schemas/wasm-trace-fixture/run.output.v1.json"
prompt_doc_ref = "prompts/wasm-trace-fixture/run.md"
"#;

    const CALLER_TRACE_TOOL_WAT: &str = r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func (result i64)))
  (type (;2;) (func (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (type (;3;) (func (param i32 i32 i32 i32 i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (import "near:agent/host@0.3.0" "log" (func $log (type 0)))
  (import "near:agent/host@0.3.0" "now-millis" (func $now (type 1)))
  (import "near:agent/host@0.3.0" "workspace-read" (func $workspace_read (type 0)))
  (import "near:agent/host@0.3.0" "http-request" (func $http_request (type 2)))
  (import "near:agent/host@0.3.0" "tool-invoke" (func $tool_invoke (type 3)))
  (import "near:agent/host@0.3.0" "secret-exists" (func $secret_exists (type 4)))
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 8192))
  (data (i32.const 1024) "{\22type\22:\22object\22}")
  (data (i32.const 2048) "caller tracing fixture")
  (data (i32.const 4096) "__LOG_BYTES__")
__ERROR_DATA__
  (func $schema (result i32)
    i32.const 16
    i32.const 1024
    i32.store
    i32.const 20
    i32.const 17
    i32.store
    i32.const 16)
  (func $description (result i32)
    i32.const 32
    i32.const 2048
    i32.store
    i32.const 36
    i32.const 22
    i32.store
    i32.const 32)
  (func $execute (param i32 i32 i32 i32 i32) (result i32)
    i32.const 2
    i32.const 4096
    i32.const __LOG_LEN__
    call $log
__OUTCOME__)
  (func $post (param i32))
  (func $realloc (param $old i32) (param $old_align i32) (param $new_size i32) (param $new_align i32) (result i32)
    (local $ret i32)
    global.get $heap
    local.set $ret
    global.get $heap
    local.get $new_size
    i32.add
    global.set $heap
    local.get $ret)
  (func $_initialize)
  (export "near:agent/tool@0.3.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.3.0#execute" (func $post))
  (export "near:agent/tool@0.3.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.3.0#schema" (func $post))
  (export "near:agent/tool@0.3.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.3.0#description" (func $post))
  (export "cabi_realloc" (func $realloc))
  (export "_initialize" (func $_initialize))
)
"#;

    fn wat_bytes(value: &str) -> String {
        value
            .as_bytes()
            .iter()
            .map(|byte| format!("\\{byte:02x}"))
            .collect()
    }

    fn caller_trace_tool_wat(log_message: &str, response_error: Option<&str>) -> String {
        let (error_data, outcome) = match response_error {
            Some(error) => (
                format!("  (data (i32.const 6144) \"{}\")", wat_bytes(error)),
                format!(
                    "    i32.const 48\n    i32.const 0\n    i32.store\n    i32.const 60\n    i32.const 1\n    i32.store\n    i32.const 64\n    i32.const 6144\n    i32.store\n    i32.const 68\n    i32.const {}\n    i32.store\n    i32.const 48",
                    error.len()
                ),
            ),
            None => (String::new(), "    unreachable".to_string()),
        };
        CALLER_TRACE_TOOL_WAT
            .replace("__LOG_BYTES__", &wat_bytes(log_message))
            .replace("__LOG_LEN__", &log_message.len().to_string())
            .replace("__ERROR_DATA__", &error_data)
            .replace("__OUTCOME__", &outcome)
    }

    fn tool_component(wat_src: &str) -> Vec<u8> {
        let mut module = wat::parse_str(wat_src).expect("fixture WAT must parse");
        let mut resolve = Resolve::default();
        let package = resolve
            .push_str("tool.wit", include_str!("../../../../wit/tool.wit"))
            .expect("tool WIT must parse");
        let world = resolve
            .select_world(&[package], Some("sandboxed-tool"))
            .expect("sandboxed-tool world must exist");
        embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8)
            .expect("component metadata must embed");
        ComponentEncoder::default()
            .module(&module)
            .expect("fixture module must decode")
            .validate(true)
            .encode()
            .expect("component must encode")
    }

    fn capability_provider_contracts() -> HostApiContractRegistry {
        let mut contracts = HostApiContractRegistry::new();
        contracts
            .register(Arc::new(
                CapabilityProviderHostApiContract::new().expect("capability provider contract"),
            ))
            .expect("register capability provider contract");
        contracts
    }

    fn caller_trace_package() -> ExtensionPackage {
        let manifest = ExtensionManifest::parse(
            CALLER_TRACE_MANIFEST,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            &capability_provider_contracts(),
        )
        .expect("trace fixture manifest should parse");
        ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new("/system/extensions/wasm-trace-fixture").unwrap(),
        )
        .expect("trace fixture package should build")
    }

    fn caller_trace_descriptor() -> CapabilityDescriptor {
        CapabilityDescriptor {
            id: caller_trace_capability_id(),
            provider: ExtensionId::new("wasm-trace-fixture").unwrap(),
            runtime: RuntimeKind::Wasm,
            trust_ceiling: TrustClass::UserTrusted,
            description: "Run tracing fixture".to_string(),
            parameters_schema: serde_json::Value::Null,
            effects: Vec::new(),
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            network_targets: Vec::new(),
            max_egress_bytes: None,
            resource_profile: None,
            origin_gate_matrix: None,
        }
    }

    fn caller_trace_capability_id() -> CapabilityId {
        CapabilityId::new("wasm-trace-fixture.run").unwrap()
    }

    fn caller_trace_policy() -> EffectiveRuntimePolicy {
        EffectiveRuntimePolicy {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalHost,
            resolved_profile: RuntimeProfile::LocalHost,
            filesystem_backend: FilesystemBackendKind::HostWorkspace,
            process_backend: ProcessBackendKind::LocalHost,
            network_mode: NetworkMode::Deny,
            secret_mode: SecretMode::Deny,
            approval_policy: ApprovalPolicy::AskDestructive,
            audit_mode: AuditMode::LocalMinimal,
        }
    }

    async fn execute_caller_trace_fixture(
        response_error: Option<&str>,
    ) -> (
        Result<super::super::RuntimeAdapterResult, super::super::DispatchError>,
        Vec<CapturedEvent>,
    ) {
        let log_message = format!("guest log {DETECTABLE_SECRET}; retained-log-context");
        let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
        let prepared = Arc::new(
            runtime
                .prepare(
                    "wasm-trace-fixture",
                    &tool_component(&caller_trace_tool_wat(&log_message, response_error)),
                )
                .unwrap(),
        );
        let package = caller_trace_package();
        let descriptor = caller_trace_descriptor();
        let filesystem = DiskFilesystem::new();
        let governor = InMemoryResourceGovernor::new();
        let policy = caller_trace_policy();
        let request = RuntimeLaneRequest {
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope: ResourceScope::system(),
            authenticated_actor_user_id: None,
            run_id: None,
            origin: None,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        };

        capture_events_async(execute_prepared_wasm(
            runtime,
            prepared,
            WitToolHost::deny_all(),
            request,
        ))
        .await
    }

    fn assert_caller_trace_envelope(events: &[CapturedEvent], capability_id: &CapabilityId) {
        assert_eq!(events.len(), 2, "guest log and error must both be traced");
        assert!(events.iter().all(|event| event.target == DIAGNOSTIC_TARGET));
        assert!(
            events
                .iter()
                .all(|event| field(event, "capability_id") == capability_id.as_str())
        );
        assert_eq!(events[0].level, Level::INFO, "guest log level must survive");
        assert_eq!(
            events[1].level,
            Level::DEBUG,
            "error event stays debug-only"
        );
        assert!(field(&events[0], "wasm_log").contains(WASM_DIAGNOSTIC_REDACTION_MARKER));
        assert!(
            events
                .iter()
                .all(|event| !format!("{event:?}").contains(DETECTABLE_SECRET)),
            "raw guest sentinel reached tracing: {events:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn production_wasm_caller_traces_sanitized_trap_and_guest_log() {
        let (result, events) = execute_caller_trace_fixture(None).await;
        let error = result.expect_err("trapping guest must fail dispatch");

        assert!(!format!("{error:?}").contains(DETECTABLE_SECRET));
        assert_caller_trace_envelope(&events, &caller_trace_capability_id());
        assert!(field(&events[1], "wasm_error").contains("unreachable"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn production_wasm_caller_traces_sanitized_response_error_and_guest_log() {
        let response_error = format!("guest response {DETECTABLE_SECRET}; retained-error-context");
        let (result, events) = execute_caller_trace_fixture(Some(&response_error)).await;
        let error = result.expect_err("guest response.error must fail dispatch");

        assert!(!format!("{error:?}").contains(DETECTABLE_SECRET));
        assert_caller_trace_envelope(&events, &caller_trace_capability_id());
        let traced_error = field(&events[1], "wasm_error");
        assert!(traced_error.contains(WASM_DIAGNOSTIC_REDACTION_MARKER));
        assert!(traced_error.contains("retained-error-context"));
    }
}
