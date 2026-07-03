use std::sync::{Arc, Mutex};

use ironclaw_extensions::{
    CapabilityProviderHostApiContract, ExtensionManifest, ExtensionPackage,
    HostApiContractRegistry, ManifestSource,
};
use ironclaw_host_api::*;
use ironclaw_resources::*;
use ironclaw_scripts::*;
use serde_json::json;

#[test]
fn script_lane_executes_manifest_command_and_reconciles_resources() {
    let backend = RecordingScriptBackend::success(ScriptBackendOutput {
        exit_code: 0,
        stdout: br#"{"message":"script ok"}"#.to_vec(),
        stderr: Vec::new(),
        wall_clock_ms: 11,
    });
    let runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), backend.clone());
    let (governor, account) = script_governor();

    let result = runtime
        .execute_extension_json(
            &governor,
            script_request(json!({"message":"hello", "command":"ignored"})),
        )
        .unwrap();

    assert_eq!(result.result.output, json!({"message":"script ok"}));
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(result.result.usage.process_count, 1);
    assert_eq!(result.result.usage.wall_clock_ms, 11);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).output_bytes > 0);

    let requests = backend.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].runner, "docker");
    assert_eq!(requests[0].image.as_deref(), Some("alpine:latest"));
    assert_eq!(requests[0].command, "script-echo");
    assert_eq!(requests[0].args, vec!["--json".to_string()]);
    let stdin_json: serde_json::Value = serde_json::from_str(&requests[0].stdin_json).unwrap();
    assert_eq!(stdin_json, json!({"message":"hello", "command":"ignored"}));
}

#[test]
fn script_lane_nonzero_exit_releases_reservation() {
    let backend = RecordingScriptBackend::success(ScriptBackendOutput {
        exit_code: 2,
        stdout: Vec::new(),
        stderr: b"raw backend detail".to_vec(),
        wall_clock_ms: 3,
    });
    let runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), backend);
    let (governor, account) = script_governor();

    let err = runtime
        .execute_extension_json(&governor, script_request(json!({"message":"fail"})))
        .unwrap_err();

    assert!(matches!(err, ScriptError::ExitFailure { code: 2, .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[test]
fn script_lane_invalid_json_releases_reservation() {
    let backend = RecordingScriptBackend::success(ScriptBackendOutput {
        exit_code: 0,
        stdout: b"not-json".to_vec(),
        stderr: Vec::new(),
        wall_clock_ms: 3,
    });
    let runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), backend);
    let (governor, account) = script_governor();

    let err = runtime
        .execute_extension_json(&governor, script_request(json!({"message":"bad-json"})))
        .unwrap_err();

    assert!(matches!(err, ScriptError::InvalidOutput { .. }));
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

    fn requests(&self) -> Vec<ScriptBackendRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl ScriptBackend for RecordingScriptBackend {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        self.requests.lock().unwrap().push(request);
        self.output.lock().unwrap().clone()
    }
}

fn script_governor() -> (InMemoryResourceGovernor, ResourceAccount) {
    let account = sample_account();
    let governor = governor_with_default_limit(account.clone());
    (governor, account)
}

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse_with_optional_host_api_contracts(
        manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    ExtensionPackage::from_manifest(manifest, root).unwrap()
}

fn governor_with_default_limit(account: ResourceAccount) -> InMemoryResourceGovernor {
    let governor = InMemoryResourceGovernor::new();
    governor
        .set_limit(
            account,
            ResourceLimits {
                max_concurrency_slots: Some(10),
                max_process_count: Some(10),
                max_output_bytes: Some(100_000),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    governor
}

fn script_request(input: serde_json::Value) -> ScriptExecutionRequest<'static> {
    let package = Box::leak(Box::new(package_from_manifest(SCRIPT_MANIFEST)));
    let capability_id = Box::leak(Box::new(CapabilityId::new("script.echo").unwrap()));
    ScriptExecutionRequest {
        package,
        capability_id,
        scope: sample_scope(),
        estimate: ResourceEstimate {
            concurrency_slots: Some(1),
            process_count: Some(1),
            output_bytes: Some(10_000),
            ..ResourceEstimate::default()
        },
        mounts: None,
        resource_reservation: None,
        invocation: ScriptInvocation { input },
    }
}

fn capability_provider_contracts() -> HostApiContractRegistry {
    let mut contracts = HostApiContractRegistry::new();
    contracts
        .register(Arc::new(CapabilityProviderHostApiContract::new().unwrap()))
        .unwrap();
    contracts
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id: InvocationId::new(),
    }
}

fn sample_account() -> ResourceAccount {
    ResourceAccount::tenant(TenantId::new("tenant-a").unwrap())
}

const SCRIPT_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "script"
name = "Script Echo"
version = "0.1.0"
description = "Script integration extension"
trust = "untrusted"

[runtime]
kind = "script"
runner = "docker"
image = "alpine:latest"
command = "script-echo"
args = ["--json"]

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "script.echo"
description = "Echo through Script"
effects = ["dispatch_capability", "execute_code"]
default_permission = "allow"
visibility = "api"
input_schema_ref = "schemas/script/echo.input.v1.json"
output_schema_ref = "schemas/script/echo.output.v1.json"
"#;
