use std::sync::{Arc, Mutex};

use ironclaw_extensions::*;
use ironclaw_host_api::*;
use ironclaw_resources::*;
use ironclaw_scripts::*;
use serde_json::json;

#[test]
fn script_runtime_reserves_executes_and_reconciles_success() {
    let backend = RecordingScriptBackend::success(ScriptBackendOutput {
        exit_code: 0,
        stdout: br#"{"message":"hello script"}"#.to_vec(),
        stderr: Vec::new(),
        wall_clock_ms: 7,
    });
    let runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), backend.clone());
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            max_process_count: Some(10),
            max_output_bytes: Some(10_000),
            ..ResourceLimits::default()
        },
    );
    let capability_id = CapabilityId::new("script.echo").unwrap();

    let execution = runtime
        .execute_extension_json(
            &governor,
            ScriptExecutionRequest {
                package: &script_package(),
                capability_id: &capability_id,
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    process_count: Some(1),
                    output_bytes: Some(10_000),
                    ..ResourceEstimate::default()
                },
                mounts: None,
                resource_reservation: None,
                invocation: ScriptInvocation {
                    input: json!({"message":"hello script", "command":"malicious override"}),
                },
            },
        )
        .unwrap();

    assert_eq!(execution.result.output, json!({"message":"hello script"}));
    assert_eq!(execution.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account).process_count, 1);
    assert!(governor.usage_for(&account).output_bytes > 0);

    let requests = backend.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].runner, "docker");
    assert_eq!(requests[0].image.as_deref(), Some("alpine:latest"));
    assert_eq!(requests[0].command, "script-echo");
    assert_eq!(requests[0].args, vec!["--json".to_string()]);
    assert_eq!(requests[0].capability_id, capability_id);
    assert!(requests[0].stdin_json.contains("hello script"));
    assert_eq!(
        requests[0].max_stdout_bytes,
        ScriptRuntimeConfig::for_testing().max_stdout_bytes
    );
    assert_eq!(
        requests[0].max_stderr_bytes,
        ScriptRuntimeConfig::for_testing().max_stderr_bytes
    );
    assert_eq!(
        requests[0].max_wall_clock_ms,
        ScriptRuntimeConfig::for_testing().max_wall_clock_ms
    );
    assert!(!requests[0].command.contains("malicious"));
}

#[test]
fn script_runtime_denies_budget_before_backend_execution() {
    let backend = RecordingScriptBackend::success(ScriptBackendOutput::json(json!({"ok": true})));
    let runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), backend.clone());
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_output_bytes: Some(1),
            ..ResourceLimits::default()
        },
    );
    let capability_id = CapabilityId::new("script.echo").unwrap();

    let err = runtime
        .execute_extension_json(
            &governor,
            ScriptExecutionRequest {
                package: &script_package(),
                capability_id: &capability_id,
                scope,
                estimate: ResourceEstimate {
                    output_bytes: Some(10_000),
                    ..ResourceEstimate::default()
                },
                mounts: None,
                resource_reservation: None,
                invocation: ScriptInvocation { input: json!({}) },
            },
        )
        .unwrap_err();

    assert!(matches!(err, ScriptError::Resource(_)));
    assert!(backend.requests.lock().unwrap().is_empty());
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[test]
fn script_runtime_releases_reservation_when_backend_exits_nonzero() {
    let backend = RecordingScriptBackend::success(ScriptBackendOutput {
        exit_code: 2,
        stdout: Vec::new(),
        stderr: b"failed".to_vec(),
        wall_clock_ms: 4,
    });
    let runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), backend);
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            ..ResourceLimits::default()
        },
    );
    let capability_id = CapabilityId::new("script.echo").unwrap();

    let err = runtime
        .execute_extension_json(
            &governor,
            ScriptExecutionRequest {
                package: &script_package(),
                capability_id: &capability_id,
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
                mounts: None,
                resource_reservation: None,
                invocation: ScriptInvocation { input: json!({}) },
            },
        )
        .unwrap_err();

    assert!(matches!(err, ScriptError::ExitFailure { code: 2, .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[test]
fn script_runtime_preserves_backend_error_when_release_cleanup_fails() {
    let backend = RecordingScriptBackend::failure("backend unavailable");
    let runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), backend);
    let governor = ReleaseFailingGovernor::new();
    let capability_id = CapabilityId::new("script.echo").unwrap();

    let err = runtime
        .execute_extension_json(
            &governor,
            ScriptExecutionRequest {
                package: &script_package(),
                capability_id: &capability_id,
                scope: sample_scope(),
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
                mounts: None,
                resource_reservation: None,
                invocation: ScriptInvocation { input: json!({}) },
            },
        )
        .unwrap_err();

    assert!(matches!(err, ScriptError::Backend { .. }));
}

#[test]
fn script_runtime_releases_reservation_when_output_limit_fails() {
    let backend = RecordingScriptBackend::success(ScriptBackendOutput {
        exit_code: 0,
        stdout: br#"{"too":"large"}"#.to_vec(),
        stderr: Vec::new(),
        wall_clock_ms: 1,
    });
    let runtime = ScriptRuntime::new(
        ScriptRuntimeConfig {
            max_stdout_bytes: 4,
            ..ScriptRuntimeConfig::for_testing()
        },
        backend,
    );
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            ..ResourceLimits::default()
        },
    );
    let capability_id = CapabilityId::new("script.echo").unwrap();

    let err = runtime
        .execute_extension_json(
            &governor,
            ScriptExecutionRequest {
                package: &script_package(),
                capability_id: &capability_id,
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
                mounts: None,
                resource_reservation: None,
                invocation: ScriptInvocation { input: json!({}) },
            },
        )
        .unwrap_err();

    assert!(matches!(err, ScriptError::OutputLimitExceeded { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[test]
fn script_runtime_rejects_non_script_package_before_reserving() {
    let backend = RecordingScriptBackend::success(ScriptBackendOutput::json(json!({"ok": true})));
    let runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), backend);
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(0),
            ..ResourceLimits::default()
        },
    );
    let capability_id = CapabilityId::new("echo.say").unwrap();

    let err = runtime
        .execute_extension_json(
            &governor,
            ScriptExecutionRequest {
                package: &wasm_package(),
                capability_id: &capability_id,
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
                mounts: None,
                resource_reservation: None,
                invocation: ScriptInvocation { input: json!({}) },
            },
        )
        .unwrap_err();

    assert!(matches!(err, ScriptError::ExtensionRuntimeMismatch { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[test]
fn script_runtime_rejects_undeclared_capability_before_reserving() {
    let backend = RecordingScriptBackend::success(ScriptBackendOutput::json(json!({"ok": true})));
    let runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), backend);
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(0),
            ..ResourceLimits::default()
        },
    );
    let capability_id = CapabilityId::new("script.missing").unwrap();

    let err = runtime
        .execute_extension_json(
            &governor,
            ScriptExecutionRequest {
                package: &script_package(),
                capability_id: &capability_id,
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
                mounts: None,
                resource_reservation: None,
                invocation: ScriptInvocation { input: json!({}) },
            },
        )
        .unwrap_err();

    assert!(matches!(err, ScriptError::CapabilityNotDeclared { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[derive(Clone)]
struct RecordingScriptBackend {
    output: Arc<Mutex<Result<ScriptBackendOutput, String>>>,
    requests: Arc<Mutex<Vec<ScriptBackendRequest>>>,
}

impl RecordingScriptBackend {
    fn success(output: ScriptBackendOutput) -> Self {
        Self {
            output: Arc::new(Mutex::new(Ok(output))),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn failure(reason: &str) -> Self {
        Self {
            output: Arc::new(Mutex::new(Err(reason.to_string()))),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ScriptBackend for RecordingScriptBackend {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        self.requests.lock().unwrap().push(request);
        self.output.lock().unwrap().clone()
    }
}

struct ReleaseFailingGovernor {
    inner: InMemoryResourceGovernor,
}

impl ReleaseFailingGovernor {
    fn new() -> Self {
        Self {
            inner: InMemoryResourceGovernor::new(),
        }
    }
}

impl ResourceGovernor for ReleaseFailingGovernor {
    fn set_limit(&self, account: ResourceAccount, limits: ResourceLimits) {
        self.inner.set_limit(account, limits);
    }

    fn reserve(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ResourceReservation, ResourceError> {
        self.inner.reserve(scope, estimate)
    }

    fn reserve_with_id(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReservation, ResourceError> {
        self.inner.reserve_with_id(scope, estimate, reservation_id)
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError> {
        self.inner.reconcile(reservation_id, actual)
    }

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, ResourceError> {
        Err(ResourceError::UnknownReservation { id: reservation_id })
    }
}

fn script_package() -> ExtensionPackage {
    package_from_manifest(SCRIPT_MANIFEST)
}

fn wasm_package() -> ExtensionPackage {
    package_from_manifest(WASM_MANIFEST)
}

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    ExtensionPackage::from_manifest(manifest, root).unwrap()
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

const SCRIPT_MANIFEST: &str = r#"
id = "script"
name = "Script Echo"
version = "0.1.0"
description = "Script demo extension"
trust = "untrusted"

[runtime]
kind = "script"
backend = "docker"
image = "alpine:latest"
command = "script-echo"
args = ["--json"]

[[capabilities]]
id = "script.echo"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const WASM_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo demo extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
