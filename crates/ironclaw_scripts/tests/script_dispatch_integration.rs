use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_dispatcher::{
    RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_resources::*;
use ironclaw_scripts::*;
use serde_json::json;

#[tokio::test]
async fn script_lane_dispatches_manifest_command_and_reconciles_through_dispatcher() {
    let backend = RecordingScriptBackend::success(ScriptBackendOutput {
        exit_code: 0,
        stdout: br#"{"message":"script ok"}"#.to_vec(),
        stderr: Vec::new(),
        wall_clock_ms: 11,
    });
    let adapter = Arc::new(ScriptRuntimeAdapter::new(backend.clone()));
    let (dispatcher, governor, events, account) = dispatcher_with_script_adapter(adapter);

    let result = dispatcher
        .dispatch_json(dispatch_request(
            json!({"message":"hello", "command":"ignored"}),
        ))
        .await
        .unwrap();

    assert_eq!(result.runtime, RuntimeKind::Script);
    assert_eq!(result.output, json!({"message":"script ok"}));
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(result.usage.process_count, 1);
    assert_eq!(result.usage.wall_clock_ms, 11);
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

    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ],
    );
}

#[tokio::test]
async fn script_lane_nonzero_exit_releases_reservation_and_emits_sanitized_failure() {
    let backend = RecordingScriptBackend::success(ScriptBackendOutput {
        exit_code: 2,
        stdout: Vec::new(),
        stderr: b"raw backend detail".to_vec(),
        wall_clock_ms: 3,
    });
    let adapter = Arc::new(ScriptRuntimeAdapter::new(backend));
    let (dispatcher, governor, events, account) = dispatcher_with_script_adapter(adapter);

    let err = dispatcher
        .dispatch_json(dispatch_request(json!({"message":"fail"})))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Script {
            kind: RuntimeDispatchErrorKind::ExitFailure
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchFailed,
        ],
    );
    let recorded = events.events();
    assert_eq!(recorded[2].error_kind.as_deref(), Some("exit_failure"));
}

#[tokio::test]
async fn script_lane_invalid_json_releases_reservation_and_emits_output_decode_failure() {
    let backend = RecordingScriptBackend::success(ScriptBackendOutput {
        exit_code: 0,
        stdout: b"not-json".to_vec(),
        stderr: Vec::new(),
        wall_clock_ms: 3,
    });
    let adapter = Arc::new(ScriptRuntimeAdapter::new(backend));
    let (dispatcher, governor, events, account) = dispatcher_with_script_adapter(adapter);

    let err = dispatcher
        .dispatch_json(dispatch_request(json!({"message":"bad-json"})))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Script {
            kind: RuntimeDispatchErrorKind::OutputDecode
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchFailed,
        ],
    );
    let recorded = events.events();
    assert_eq!(recorded[2].error_kind.as_deref(), Some("output_decode"));
}

#[derive(Clone)]
struct ScriptRuntimeAdapter<B> {
    runtime: ScriptRuntime<B>,
}

impl<B> ScriptRuntimeAdapter<B>
where
    B: ScriptBackend,
{
    fn new(backend: B) -> Self {
        Self {
            runtime: ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), backend),
        }
    }
}

#[async_trait]
impl<B> RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> for ScriptRuntimeAdapter<B>
where
    B: ScriptBackend + Send + Sync,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let execution = self
            .runtime
            .execute_extension_json(
                request.governor,
                ScriptExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    mounts: None,
                    resource_reservation: request.resource_reservation,
                    invocation: ScriptInvocation {
                        input: request.input,
                    },
                },
            )
            .map_err(|error| DispatchError::Script {
                kind: script_error_kind(&error),
            })?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
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

fn dispatcher_with_script_adapter<T>(
    adapter: Arc<T>,
) -> (
    RuntimeDispatcher<'static, LocalFilesystem, InMemoryResourceGovernor>,
    Arc<InMemoryResourceGovernor>,
    InMemoryEventSink,
    ResourceAccount,
)
where
    T: RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> + 'static,
{
    let account = sample_account();
    let registry = Arc::new(registry_with_package(SCRIPT_MANIFEST));
    let filesystem = Arc::new(mounted_empty_extension_root());
    let governor = Arc::new(governor_with_default_limit(account.clone()));
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::from_arcs(registry, filesystem, Arc::clone(&governor))
        .with_runtime_adapter_arc(RuntimeKind::Script, adapter)
        .with_event_sink_arc(Arc::new(events.clone()));
    (dispatcher, governor, events, account)
}

fn registry_with_package(manifest: &str) -> ironclaw_extensions::ExtensionRegistry {
    let mut registry = ironclaw_extensions::ExtensionRegistry::new();
    registry.insert(package_from_manifest(manifest)).unwrap();
    registry
}

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    ExtensionPackage::from_manifest(manifest, root).unwrap()
}

fn mounted_empty_extension_root() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

fn governor_with_default_limit(account: ResourceAccount) -> InMemoryResourceGovernor {
    let governor = InMemoryResourceGovernor::new();
    governor.set_limit(
        account,
        ResourceLimits {
            max_concurrency_slots: Some(10),
            max_process_count: Some(10),
            max_output_bytes: Some(100_000),
            ..ResourceLimits::default()
        },
    );
    governor
}

fn dispatch_request(input: serde_json::Value) -> CapabilityDispatchRequest {
    CapabilityDispatchRequest {
        capability_id: CapabilityId::new("script.echo").unwrap(),
        scope: sample_scope(),
        estimate: ResourceEstimate {
            concurrency_slots: Some(1),
            process_count: Some(1),
            output_bytes: Some(10_000),
            ..ResourceEstimate::default()
        },
        mounts: None,
        resource_reservation: None,
        input,
    }
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

fn assert_event_kinds(events: &InMemoryEventSink, expected: &[RuntimeEventKind]) {
    let actual = events
        .events()
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

fn script_error_kind(error: &ScriptError) -> RuntimeDispatchErrorKind {
    match error {
        ScriptError::Resource(_) => RuntimeDispatchErrorKind::Resource,
        ScriptError::Backend { .. } => RuntimeDispatchErrorKind::Backend,
        ScriptError::UnsupportedRunner { .. } => RuntimeDispatchErrorKind::UnsupportedRunner,
        ScriptError::ExtensionRuntimeMismatch { .. } => {
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
        }
        ScriptError::CapabilityNotDeclared { .. } => RuntimeDispatchErrorKind::UndeclaredCapability,
        ScriptError::DescriptorMismatch { .. } => RuntimeDispatchErrorKind::Manifest,
        ScriptError::InvalidInvocation { .. } => RuntimeDispatchErrorKind::InputEncode,
        ScriptError::ExitFailure { .. } => RuntimeDispatchErrorKind::ExitFailure,
        ScriptError::OutputLimitExceeded { .. } => RuntimeDispatchErrorKind::OutputTooLarge,
        ScriptError::Timeout { .. } => RuntimeDispatchErrorKind::Executor,
        ScriptError::InvalidOutput { .. } => RuntimeDispatchErrorKind::OutputDecode,
    }
}

const SCRIPT_MANIFEST: &str = r#"
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

[[capabilities]]
id = "script.echo"
description = "Echo through Script"
effects = ["dispatch_capability", "execute_code"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
