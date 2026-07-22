use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_dispatcher::{
    BoundCapabilityAdapter, CapabilityDispatchRequest, CapabilityDispatcher, DispatchError,
    ResolvedCapability, RuntimeAdapterResult, RuntimeDispatchErrorKind, RuntimeDispatcher,
    ToolResolver,
};
use ironclaw_extensions::{
    CapabilityVisibility, ExtensionError, ExtensionLifecycleService, ExtensionManifest,
    ExtensionPackage, ExtensionRegistry, ExtensionRuntime, ManifestSource, ManifestV2Error,
};
use ironclaw_filesystem::DiskFilesystem;
use ironclaw_host_api::{
    ActivityId, Actor, Authorized, CapabilityId, CorrelationId, EffectKind, ExtensionId, HostPath,
    Invocation, InvocationOrigin, MountView, NetworkScheme, NetworkTargetPattern, PermissionMode,
    ProcessId, ProductKind, ReservationStatus, ResourceEstimate, ResourceReservationId,
    ResourceScope, ResourceUsage, RuntimeCredentialRequirementSource, RuntimeCredentialTarget,
    RuntimeKind, RuntimeLane, SecretHandle, TenantId, Timestamp, UserId, VendorId, VirtualPath,
};
use ironclaw_host_runtime::{
    default_host_api_contract_registry, default_host_port_catalog,
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
    let estimate = ResourceEstimate::default()
        .set_concurrency_slots(1)
        .set_process_count(1)
        .set_output_bytes(10_000);
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_concurrency_slots(1)
                .set_max_process_count(1)
                .set_max_output_bytes(10_000),
        )
        .unwrap();
    let adapter = Arc::new(RecordingAdapter::new(
        RuntimeKind::Script,
        json!({"message":"script ok"}),
        Arc::clone(&governor),
    ));
    // The registry-lane resolver's selection semantics are pinned in
    // `ironclaw_host_runtime::services` tests; this e2e drives the dispatch
    // flow through a binding scripted from the discovered descriptor.
    let descriptor = discovered
        .get_capability(&CapabilityId::new("script.echo").unwrap())
        .unwrap();
    let resolver: Arc<dyn ToolResolver> = Arc::new(SingleCapabilityResolver {
        capability_id: descriptor.id.clone(),
        resolved: ResolvedCapability {
            provider: descriptor.provider.clone(),
            runtime: descriptor.runtime,
            adapter: Arc::clone(&adapter) as Arc<dyn BoundCapabilityAdapter>,
        },
    });
    let dispatcher = RuntimeDispatcher::from_arcs(resolver, Arc::clone(&governor));
    let dispatch_port: &dyn CapabilityDispatcher = &dispatcher;
    let reservation = governor.reserve(scope.clone(), estimate.clone()).unwrap();
    let reservation_id = reservation.id;
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 1);

    let result = dispatch_port
        .dispatch_json(Authorized::seal_for_test(
            Invocation {
                activity_id: ActivityId::new(),
                capability: CapabilityId::new("script.echo").unwrap(),
                input: json!({"message":"hello"}),
                scope: scope.clone(),
                actor: Actor::System,
                origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
                estimate: estimate.clone(),
                correlation_id: CorrelationId::new(),
                process_id: Some(ProcessId::new()),
                parent_process_id: None,
            },
            RuntimeLane::Process,
            MountView::default(),
            Some(reservation),
            Timestamp::MAX_UTC,
        ))
        .await
        .unwrap();

    assert_eq!(result.output, json!({"message":"script ok"}));
    assert_eq!(result.provider, extension_id);
    assert_eq!(result.runtime, RuntimeKind::Script);
    assert_eq!(result.receipt.id, reservation_id);
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).output_bytes > 0);

    let requests = adapter.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].capability_id,
        CapabilityId::new("script.echo").unwrap()
    );
    assert_eq!(requests[0].scope, scope);
    assert_eq!(requests[0].estimate, estimate);
    assert_eq!(requests[0].mounts, Some(MountView::default()));
    assert_eq!(requests[0].resource_reservation_id, Some(reservation_id));
    assert_eq!(requests[0].input, json!({"message":"hello"}));
}

#[tokio::test]
async fn github_v2_package_discovers_and_publishes_issue_hot_catalog() {
    let github_asset_root = github_first_party_asset_root();
    assert!(github_asset_root.join("wasm-src/Cargo.toml").is_file());

    let (_storage, fs) = mounted_github_package_fs();
    // Parse through the single record entry point (the github asset is a
    // manifest v3 document).
    let record = ironclaw_extensions::ExtensionManifestRecord::from_toml(
        std::fs::read_to_string(github_asset_root.join("manifest.toml")).unwrap(),
        ManifestSource::HostBundled,
        &default_host_port_catalog().unwrap(),
        None,
        &default_host_api_contract_registry().unwrap(),
    )
    .unwrap();
    let manifest = ExtensionManifest::try_from(record.manifest().clone()).unwrap();
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/github").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let extension_id = ExtensionId::new("github").unwrap();
    let package = registry.get_extension(&extension_id).unwrap();

    assert!(matches!(
        &package.manifest.runtime,
        ExtensionRuntime::Wasm { module } if module.as_str() == "wasm/github_tool.wasm"
    ));
    let expected_github_capability_ids = [
        "github.get_repo",
        "github.create_repo",
        "github.list_issues",
        "github.create_issue",
        "github.update_issue",
        "github.add_issue_labels",
        "github.remove_issue_label",
        "github.add_issue_assignees",
        "github.remove_issue_assignees",
        "github.get_issue",
        "github.list_issue_comments",
        "github.create_issue_comment",
        "github.comment_issue",
        "github.list_pull_requests",
        "github.create_pull_request",
        "github.update_pull_request",
        "github.get_pull_request",
        "github.get_pull_request_files",
        "github.create_pr_review",
        "github.list_pull_request_comments",
        "github.reply_pull_request_comment",
        "github.get_pull_request_reviews",
        "github.list_pull_request_review_threads",
        "github.resolve_review_thread",
        "github.unresolve_review_thread",
        "github.get_combined_status",
        "github.merge_pull_request",
        "github.get_authenticated_user",
        "github.list_repos",
        "github.search_repositories",
        "github.search_code",
        "github.search_issues",
        "github.search_issues_pull_requests",
        "github.list_branches",
        "github.create_branch",
        "github.get_file_content",
        "github.create_or_update_file",
        "github.delete_file",
        "github.list_releases",
        "github.create_release",
        "github.trigger_workflow",
        "github.get_workflow_runs",
        "github.get_workflow_run_jobs",
        "github.get_job_logs",
        "github.get_workflow_run_artifacts",
        "github.rerun_failed_workflow_run_jobs",
        "github.rerun_workflow_job",
        "github.fork_repo",
        "github.handle_webhook",
    ];
    assert_eq!(expected_github_capability_ids.len(), 49);
    assert_eq!(
        package
            .capabilities
            .iter()
            .map(|capability| capability.id.as_str())
            .collect::<Vec<_>>()
            .as_slice(),
        expected_github_capability_ids
    );
    assert_eq!(
        package
            .manifest
            .capabilities
            .iter()
            .map(|capability| capability.id.as_str())
            .collect::<Vec<_>>()
            .as_slice(),
        expected_github_capability_ids
    );
    let model_visible_capability_ids = package
        .manifest
        .capabilities
        .iter()
        .filter(|capability| capability.visibility == CapabilityVisibility::Model)
        .map(|capability| capability.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        model_visible_capability_ids.as_slice(),
        expected_github_capability_ids
    );
    for (capability_id, expected_effects, expected_permission, expects_github_api_access) in [
        (
            "github.get_repo",
            // The v3 normalizer adds the dispatch effect uniformly (v2
            // declared it inconsistently across the github tools).
            vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::UseSecret,
            ],
            PermissionMode::Allow,
            true,
        ),
        (
            "github.create_repo",
            vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::UseSecret,
                EffectKind::ExternalWrite,
            ],
            PermissionMode::Ask,
            true,
        ),
        (
            "github.handle_webhook",
            vec![EffectKind::DispatchCapability],
            PermissionMode::Ask,
            false,
        ),
    ] {
        let capability = package
            .manifest
            .capabilities
            .iter()
            .find(|capability| capability.id.as_str() == capability_id)
            .unwrap();
        assert_eq!(capability.effects, expected_effects);
        assert_eq!(capability.default_permission, expected_permission);
        if expects_github_api_access {
            assert_eq!(
                capability
                    .required_host_ports
                    .iter()
                    .map(|port| port.as_str())
                    .collect::<Vec<_>>(),
                vec!["host.runtime.http_egress"]
            );
            assert_eq!(capability.runtime_credentials.len(), 1);
            let credential = &capability.runtime_credentials[0];
            assert_eq!(
                credential.handle,
                SecretHandle::new("github_runtime_token").unwrap()
            );
            assert_eq!(
                credential.source,
                RuntimeCredentialRequirementSource::ProductAuthAccount {
                    provider: VendorId::new("github").unwrap(),
                    setup: Default::default(),
                }
            );
            assert_eq!(
                credential.audience,
                NetworkTargetPattern {
                    scheme: Some(NetworkScheme::Https),
                    host_pattern: "api.github.com".to_string(),
                    port: None,
                }
            );
            assert_eq!(
                credential.target,
                RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                }
            );
            assert!(credential.required);
        } else {
            assert!(capability.required_host_ports.is_empty());
            assert!(capability.runtime_credentials.is_empty());
        }
    }

    let hot_catalog = publish_hot_capability_catalog(&fs, &registry)
        .await
        .unwrap();
    assert_eq!(
        hot_catalog
            .capabilities
            .iter()
            .map(|capability| capability.descriptor.id.as_str())
            .collect::<Vec<_>>()
            .as_slice(),
        expected_github_capability_ids
    );
    assert_eq!(hot_catalog.capabilities.len(), 49);

    let search = hot_catalog
        .get(&CapabilityId::new("github.search_issues").unwrap())
        .unwrap();
    assert_eq!(search.descriptor.provider, extension_id);
    assert_eq!(search.descriptor.runtime, RuntimeKind::Wasm);
    assert_eq!(
        search.descriptor.parameters_schema["properties"]["query"]["type"],
        json!("string")
    );
    // Manifest v3 declares no output schema; the hot catalog treats the
    // output as unconstrained.
    assert_eq!(search.output_schema, json!({}));
    assert!(
        search
            .prompt_doc
            .as_deref()
            .is_some_and(|doc| doc.contains("github.search_issues")
                && doc.contains("GitHub product-auth account"))
    );

    let get_issue = hot_catalog
        .get(&CapabilityId::new("github.get_issue").unwrap())
        .unwrap();
    assert_eq!(
        get_issue.descriptor.parameters_schema["required"],
        json!(["owner", "repo", "issue_number"])
    );
    assert_eq!(
        get_issue.descriptor.parameters_schema["properties"]["owner"]["pattern"],
        json!("^[^\\s/?#]+$")
    );
    assert_eq!(
        get_issue.descriptor.parameters_schema["properties"]["owner"]["not"]["pattern"],
        json!("\\.\\.")
    );
    assert_eq!(get_issue.output_schema, json!({}));
    assert!(
        get_issue
            .prompt_doc
            .as_deref()
            .is_some_and(|doc| doc.contains("github.get_issue")
                && doc.contains("reads from the GitHub API")
                && doc.contains("configured GitHub product-auth account"))
    );

    let comment_issue = hot_catalog
        .get(&CapabilityId::new("github.comment_issue").unwrap())
        .unwrap();
    assert_eq!(
        comment_issue.descriptor.parameters_schema["required"],
        json!(["owner", "repo", "issue_number", "body"])
    );
    assert_eq!(
        comment_issue.descriptor.effects,
        vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::UseSecret,
            EffectKind::ExternalWrite,
        ]
    );
    assert_eq!(comment_issue.output_schema, json!({}));
    assert!(comment_issue.prompt_doc.as_deref().is_some_and(|doc| {
        doc.contains("github.comment_issue")
            && doc.contains("external write")
            && doc.contains("GitHub product-auth account")
    }));
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

    // The capability-provider contract preserves typed manifest errors
    // (`HostApiSectionError::Manifest` unwraps back to the precise variant),
    // so the unknown port surfaces as `UnknownHostPort`, still fail-closed
    // before install.
    assert!(
        matches!(
            err,
            ExtensionError::ManifestV2(ManifestV2Error::UnknownHostPort { ref port, .. })
                if port.as_str() == "host.runtime.not_supported"
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
    governor: Arc<InMemoryResourceGovernor>,
    requests: Arc<Mutex<Vec<RecordedAdapterRequest>>>,
}

impl RecordingAdapter {
    fn new(runtime: RuntimeKind, output: Value, governor: Arc<InMemoryResourceGovernor>) -> Self {
        Self {
            runtime,
            output,
            governor,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<RecordedAdapterRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RecordedAdapterRequest {
    capability_id: CapabilityId,
    scope: ResourceScope,
    estimate: ResourceEstimate,
    mounts: Option<MountView>,
    resource_reservation_id: Option<ResourceReservationId>,
    input: Value,
}

struct SingleCapabilityResolver {
    capability_id: CapabilityId,
    resolved: ResolvedCapability,
}

impl ToolResolver for SingleCapabilityResolver {
    fn resolve(&self, capability_id: &CapabilityId) -> Option<ResolvedCapability> {
        (capability_id == &self.capability_id).then(|| self.resolved.clone())
    }
}

#[async_trait]
impl BoundCapabilityAdapter for RecordingAdapter {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        self.requests.lock().unwrap().push(RecordedAdapterRequest {
            capability_id: request.capability_id.clone(),
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
        let usage = ResourceUsage::default()
            .set_output_bytes(output_bytes)
            .set_process_count(u32::from(matches!(
                self.runtime,
                RuntimeKind::Script | RuntimeKind::Mcp
            )));
        let reservation = match request.resource_reservation {
            Some(reservation) => reservation,
            None => self
                .governor
                .reserve(request.scope, request.estimate)
                .map_err(|_| {
                    dispatch_error_for_runtime(self.runtime, RuntimeDispatchErrorKind::Resource)
                })?,
        };
        let receipt = self
            .governor
            .reconcile(reservation.id, usage.clone())
            .map_err(|_| {
                dispatch_error_for_runtime(self.runtime, RuntimeDispatchErrorKind::Resource)
            })?;

        Ok(RuntimeAdapterResult {
            output: self.output.clone(),
            display_preview: None,
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
        RuntimeKind::Script => DispatchError::Script {
            kind,
            model_visible_cause: None,
        },
        RuntimeKind::Wasm => DispatchError::Wasm {
            kind,
            model_visible_cause: None,
        },
        RuntimeKind::Mcp => DispatchError::Mcp {
            kind,
            model_visible_cause: None,
        },
        RuntimeKind::FirstParty | RuntimeKind::System => DispatchError::UnsupportedRuntime {
            capability: CapabilityId::new("system.unsupported").unwrap(),
            runtime,
        },
    }
}

fn mounted_extension_fs(id: &str, manifest: &str) -> (tempfile::TempDir, DiskFilesystem) {
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

    let mut fs = DiskFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    (storage, fs)
}

fn mounted_github_package_fs() -> (tempfile::TempDir, DiskFilesystem) {
    let storage = tempdir().unwrap();
    let source_root = github_first_party_asset_root();
    let package_root = storage.path().join("github");

    copy_package_file(&source_root, &package_root, "manifest.toml");
    copy_package_dir(
        &source_root.join("schemas/github"),
        &package_root.join("schemas/github"),
    );
    copy_package_dir(
        &source_root.join("prompts/github"),
        &package_root.join("prompts/github"),
    );

    let mut fs = DiskFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    (storage, fs)
}

fn github_first_party_asset_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("crates/ironclaw_first_party_extensions/assets/github")
}

fn copy_package_file(source_root: &Path, package_root: &Path, relative: &str) {
    let source = source_root.join(relative);
    let destination = package_root.join(relative);
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::copy(source, destination).unwrap();
}

fn copy_package_dir(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_package_dir(&source_path, &destination_path);
        } else {
            std::fs::copy(source_path, destination_path).unwrap();
        }
    }
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
