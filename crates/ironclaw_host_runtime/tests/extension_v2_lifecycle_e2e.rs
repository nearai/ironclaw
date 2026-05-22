use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_dispatcher::{
    CapabilityDispatcher, DispatchError, RuntimeAdapter, RuntimeAdapterRequest,
    RuntimeAdapterResult, RuntimeDispatchErrorKind, RuntimeDispatcher,
};
use ironclaw_extensions::{
    ExtensionError, ExtensionLifecycleService, ExtensionRegistry, ExtensionRuntime, ManifestV2Error,
};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{
    CapabilityId, EffectKind, ExtensionId, HostPath, MountView, PermissionMode, ReservationStatus,
    ResourceEstimate, ResourceReservationId, ResourceScope, ResourceUsage, RuntimeKind, TenantId,
    UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    discover_extensions_with_default_host_api_contracts, publish_hot_capability_catalog,
};
use ironclaw_resources::{
    InMemoryResourceGovernor, ResourceAccount, ResourceGovernor, ResourceLimits, ResourceTally,
};
use serde_json::{Value, json};
use tempfile::tempdir;

#[tokio::test]
async fn extension_v2_lifecycle_discovers_installs_publishes_and_dispatches_host_api_capability() {
    let (_storage, fs) = mounted_extension_fs("script", SCRIPT_MANIFEST);
    let discovered = discover_extensions_with_default_host_api_contracts(
        &fs,
        &VirtualPath::new("/system/extensions").unwrap(),
    )
    .await
    .unwrap();
    let package = discovered
        .get_extension(&ExtensionId::new("script").unwrap())
        .unwrap()
        .clone();

    let mut lifecycle = ExtensionLifecycleService::new(ExtensionRegistry::new());
    lifecycle.install(package).await.unwrap();
    let extension_id = ExtensionId::new("script").unwrap();
    assert!(lifecycle.is_enabled(&extension_id));
    lifecycle.disable(&extension_id).await.unwrap();
    assert!(!lifecycle.is_enabled(&extension_id));
    lifecycle.enable(&extension_id).await.unwrap();
    assert!(lifecycle.is_enabled(&extension_id));

    let hot_catalog = publish_hot_capability_catalog(&fs, lifecycle.registry())
        .await
        .unwrap();
    let hot_record = hot_catalog
        .get(&CapabilityId::new("script.echo").unwrap())
        .unwrap();
    assert_eq!(
        hot_record.descriptor.parameters_schema,
        json!({"type":"object","properties":{"message":{"type":"string"}},"required":["message"]})
    );
    assert_eq!(hot_record.output_schema, json!({"type":"object"}));
    assert_eq!(
        hot_record.prompt_doc.as_deref(),
        Some("Echo user-provided text through the script runtime.")
    );

    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let estimate = ResourceEstimate {
        concurrency_slots: Some(1),
        process_count: Some(1),
        output_bytes: Some(10_000),
        ..ResourceEstimate::default()
    };
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_concurrency_slots: Some(1),
                max_process_count: Some(1),
                max_output_bytes: Some(10_000),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    let adapter = Arc::new(RecordingAdapter::new(
        RuntimeKind::Script,
        json!({"message":"script ok"}),
    ));
    let dispatcher =
        RuntimeDispatcher::from_arcs(Arc::new(discovered), Arc::new(fs), Arc::clone(&governor))
            .with_runtime_adapter_arc(RuntimeKind::Script, Arc::clone(&adapter));
    let dispatch_port: &dyn CapabilityDispatcher = &dispatcher;
    let reservation = governor.reserve(scope.clone(), estimate.clone()).unwrap();
    let reservation_id = reservation.id;
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 1);

    let result = dispatch_port
        .dispatch_json(ironclaw_host_api::CapabilityDispatchRequest {
            capability_id: CapabilityId::new("script.echo").unwrap(),
            scope: scope.clone(),
            estimate: estimate.clone(),
            mounts: None,
            resource_reservation: Some(reservation),
            input: json!({"message":"hello"}),
        })
        .await
        .unwrap();

    assert_eq!(result.output, json!({"message":"script ok"}));
    assert_eq!(result.receipt.id, reservation_id);
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).output_bytes > 0);

    let requests = adapter.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].provider, extension_id);
    assert_eq!(
        requests[0].capability_id,
        CapabilityId::new("script.echo").unwrap()
    );
    assert_eq!(requests[0].runtime, RuntimeKind::Script);
    assert_eq!(requests[0].scope, scope);
    assert_eq!(requests[0].estimate, estimate);
    assert_eq!(requests[0].mounts, None);
    assert_eq!(requests[0].resource_reservation_id, Some(reservation_id));
    assert_eq!(requests[0].input, json!({"message":"hello"}));
}

#[tokio::test]
async fn github_v2_package_discovers_and_publishes_read_only_hot_catalog() {
    let (_storage, fs) = mounted_github_package_fs();
    let registry = discover_extensions_with_default_host_api_contracts(
        &fs,
        &VirtualPath::new("/system/extensions").unwrap(),
    )
    .await
    .unwrap();
    let extension_id = ExtensionId::new("github").unwrap();
    let package = registry.get_extension(&extension_id).unwrap();

    assert!(matches!(
        &package.manifest.runtime,
        ExtensionRuntime::Wasm { module } if module.as_str() == "wasm/github_tool.wasm"
    ));
    assert_eq!(
        package
            .capabilities
            .iter()
            .map(|capability| capability.id.as_str())
            .collect::<Vec<_>>(),
        vec!["github.search_issues", "github.get_issue"]
    );
    for capability in &package.manifest.capabilities {
        assert_eq!(
            capability.effects,
            vec![EffectKind::Network, EffectKind::UseSecret]
        );
        assert_eq!(capability.default_permission, PermissionMode::Ask);
        assert_eq!(
            capability
                .required_host_ports
                .iter()
                .map(|port| port.as_str())
                .collect::<Vec<_>>(),
            vec!["host.runtime.http_egress"]
        );
    }

    let hot_catalog = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap();
    assert_eq!(hot_catalog.capabilities.len(), 2);

    let search = hot_catalog
        .get(&CapabilityId::new("github.search_issues").unwrap())
        .unwrap();
    assert_eq!(search.descriptor.provider, extension_id);
    assert_eq!(search.descriptor.runtime, RuntimeKind::Wasm);
    assert_eq!(
        search.descriptor.parameters_schema["properties"]["query"]["type"],
        json!("string")
    );
    assert_eq!(
        search.output_schema["properties"]["items"]["type"],
        json!("array")
    );
    assert!(search
        .prompt_doc
        .as_deref()
        .is_some_and(|doc| doc.contains("github.search_issues")
            && doc.contains("github_token")));

    let get_issue = hot_catalog
        .get(&CapabilityId::new("github.get_issue").unwrap())
        .unwrap();
    assert_eq!(
        get_issue.descriptor.parameters_schema["required"],
        json!(["owner", "repo", "issue_number"])
    );
    assert_eq!(
        get_issue.output_schema["required"],
        json!(["number", "title", "state", "html_url"])
    );
    assert!(
        get_issue
            .prompt_doc
            .as_deref()
            .is_some_and(|doc| doc.contains("github.get_issue") && doc.contains("read-only"))
    );
}

#[tokio::test]
async fn extension_v2_lifecycle_fails_closed_before_install_for_unknown_required_host_port() {
    let manifest =
        SCRIPT_MANIFEST.replace("host.runtime.http_egress", "host.runtime.not_supported");
    let (_storage, fs) = mounted_extension_fs("script", &manifest);

    let err = discover_extensions_with_default_host_api_contracts(
        &fs,
        &VirtualPath::new("/system/extensions").unwrap(),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(
            err,
            ExtensionError::ManifestV2(ManifestV2Error::HostApiSectionRejected { ref reason, .. })
                if reason.contains("unknown host port 'host.runtime.not_supported'")
        ),
        "unexpected error: {err:?}"
    );
    let lifecycle = ExtensionLifecycleService::new(ExtensionRegistry::new());
    assert!(!lifecycle.is_enabled(&ExtensionId::new("script").unwrap()));
}

#[derive(Clone)]
struct RecordingAdapter {
    runtime: RuntimeKind,
    output: Value,
    requests: Arc<Mutex<Vec<RecordedAdapterRequest>>>,
}

impl RecordingAdapter {
    fn new(runtime: RuntimeKind, output: Value) -> Self {
        Self {
            runtime,
            output,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<RecordedAdapterRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RecordedAdapterRequest {
    provider: ExtensionId,
    capability_id: CapabilityId,
    runtime: RuntimeKind,
    scope: ResourceScope,
    estimate: ResourceEstimate,
    mounts: Option<MountView>,
    resource_reservation_id: Option<ResourceReservationId>,
    input: Value,
}

#[async_trait]
impl RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> for RecordingAdapter {
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        self.requests.lock().unwrap().push(RecordedAdapterRequest {
            provider: request.package.id.clone(),
            capability_id: request.capability_id.clone(),
            runtime: request.descriptor.runtime,
            scope: request.scope.clone(),
            estimate: request.estimate.clone(),
            mounts: request.mounts.clone(),
            resource_reservation_id: request
                .resource_reservation
                .as_ref()
                .map(|reservation| reservation.id),
            input: request.input.clone(),
        });

        let output_bytes = serde_json::to_vec(&self.output).unwrap().len() as u64;
        let usage = ResourceUsage {
            output_bytes,
            process_count: u32::from(matches!(
                self.runtime,
                RuntimeKind::Script | RuntimeKind::Mcp
            )),
            ..ResourceUsage::default()
        };
        let reservation = match request.resource_reservation {
            Some(reservation) => reservation,
            None => request
                .governor
                .reserve(request.scope, request.estimate)
                .map_err(|_| {
                    dispatch_error_for_runtime(self.runtime, RuntimeDispatchErrorKind::Resource)
                })?,
        };
        let receipt = request
            .governor
            .reconcile(reservation.id, usage.clone())
            .map_err(|_| {
                dispatch_error_for_runtime(self.runtime, RuntimeDispatchErrorKind::Resource)
            })?;

        Ok(RuntimeAdapterResult {
            output: self.output.clone(),
            usage,
            receipt,
            output_bytes,
        })
    }
}

fn dispatch_error_for_runtime(
    runtime: RuntimeKind,
    kind: RuntimeDispatchErrorKind,
) -> DispatchError {
    match runtime {
        RuntimeKind::Script => DispatchError::Script { kind },
        RuntimeKind::Wasm => DispatchError::Wasm { kind },
        RuntimeKind::Mcp => DispatchError::Mcp { kind },
        RuntimeKind::FirstParty | RuntimeKind::System => DispatchError::UnsupportedRuntime {
            capability: CapabilityId::new("system.unsupported").unwrap(),
            runtime,
        },
    }
}

fn mounted_extension_fs(id: &str, manifest: &str) -> (tempfile::TempDir, LocalFilesystem) {
    let storage = tempdir().unwrap();
    let extension_root = storage.path().join(id);
    std::fs::create_dir_all(extension_root.join("schemas/script")).unwrap();
    std::fs::create_dir_all(extension_root.join("prompts/script")).unwrap();
    std::fs::write(extension_root.join("manifest.toml"), manifest).unwrap();
    std::fs::write(
        extension_root.join("schemas/script/echo.input.v1.json"),
        r#"{"type":"object","properties":{"message":{"type":"string"}},"required":["message"]}"#,
    )
    .unwrap();
    std::fs::write(
        extension_root.join("schemas/script/echo.output.v1.json"),
        r#"{"type":"object"}"#,
    )
    .unwrap();
    std::fs::write(
        extension_root.join("prompts/script/echo.md"),
        "Echo user-provided text through the script runtime.",
    )
    .unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    (storage, fs)
}

fn mounted_github_package_fs() -> (tempfile::TempDir, LocalFilesystem) {
    let storage = tempdir().unwrap();
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tools-src/github-reborn");
    let package_root = storage.path().join("github");

    for relative in [
        "manifest.toml",
        "schemas/github/search_issues.input.v1.json",
        "schemas/github/search_issues.output.v1.json",
        "schemas/github/get_issue.input.v1.json",
        "schemas/github/get_issue.output.v1.json",
        "prompts/github/search_issues.md",
        "prompts/github/get_issue.md",
    ] {
        copy_package_file(&source_root, &package_root, relative);
    }

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    (storage, fs)
}

fn copy_package_file(source_root: &Path, package_root: &Path, relative: &str) {
    let source = source_root.join(relative);
    let destination = package_root.join(relative);
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::copy(source, destination).unwrap();
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::InvocationId::new(),
    }
}

const SCRIPT_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "script"
name = "Script Echo"
version = "0.1.0"
description = "Script lifecycle extension"
trust = "third_party"

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
visibility = "model"
input_schema_ref = "schemas/script/echo.input.v1.json"
output_schema_ref = "schemas/script/echo.output.v1.json"
prompt_doc_ref = "prompts/script/echo.md"
required_host_ports = ["host.runtime.http_egress"]
"#;
