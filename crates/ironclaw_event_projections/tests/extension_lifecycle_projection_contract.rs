use std::{fs, path::Path, sync::Arc};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_event_projections::{
    AuditProjectionRequest, AuditProjectionService, AuditProjectionStage, ProjectionScope,
    ReplayAuditProjectionService,
};
use ironclaw_events::{AuditSink, DurableAuditSink, EventError};
use ironclaw_extensions::{
    ExtensionError, ExtensionLifecycleEvent, ExtensionLifecycleEventSink,
    ExtensionLifecycleService, ExtensionManifest, ExtensionPackage, ExtensionRegistry,
    ManifestSource,
};
use ironclaw_host_api::{
    ActionSummary, AgentId, AuditEnvelope, AuditEventId, AuditStage, CorrelationId,
    DecisionSummary, EffectKind, ExtensionId, ExtensionLifecycleOperation, HostPortCatalog,
    InvocationId, ProjectId, ResourceScope, TenantId, UserId, VirtualPath,
};
use ironclaw_reborn_event_store::{
    RebornEventStoreConfig, RebornProfile, build_reborn_event_stores,
};

#[tokio::test]
async fn extension_lifecycle_projects_metadata_only_from_durable_audit_log() {
    let temp = tempfile::tempdir().unwrap();
    let store_root = temp.path().join("reborn-event-store");
    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: store_root.clone(),
            accept_single_node_durable: false,
        },
    )
    .await
    .unwrap();
    let audit_log = Arc::clone(&stores.audit);
    let lifecycle_sink = Arc::new(DurableExtensionLifecycleAuditSink::new(Arc::new(
        DurableAuditSink::new(Arc::clone(&audit_log)),
    )));
    let package = ExtensionPackage::from_manifest(
        ExtensionManifest::parse(
            EXTENSION_MANIFEST_WITH_RAW_SENTINELS,
            ManifestSource::InstalledLocal,
            &HostPortCatalog::empty(),
        )
        .unwrap(),
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    let updated_package = ExtensionPackage::from_manifest(
        ExtensionManifest::parse(
            &EXTENSION_MANIFEST_WITH_RAW_SENTINELS
                .replace("version = \"0.1.0\"", "version = \"0.2.0\"")
                .replace("echo.say", "echo.reply"),
            ManifestSource::InstalledLocal,
            &HostPortCatalog::empty(),
        )
        .unwrap(),
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    let mut service =
        ExtensionLifecycleService::new(ExtensionRegistry::new()).with_event_sink(lifecycle_sink);

    let extension_id = ExtensionId::new("echo").unwrap();
    service.install(package).await.unwrap();
    service.update(updated_package).await.unwrap();
    service.disable(&extension_id).await.unwrap();
    service.enable(&extension_id).await.unwrap();
    service.remove(&extension_id).await.unwrap();

    assert!(service.registry().get_extension(&extension_id).is_none());
    let projection = ReplayAuditProjectionService::from_audit_log(Arc::clone(&audit_log));
    let snapshot = projection
        .snapshot(AuditProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&extension_resource_scope()),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.entries.len(), 5);
    let decision_kinds = snapshot
        .entries
        .iter()
        .map(|entry| entry.decision_kind.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        decision_kinds,
        vec![
            "extension_installed",
            "extension_updated",
            "extension_disabled",
            "extension_enabled",
            "extension_removed",
        ]
    );
    assert!(snapshot.entries.iter().all(|entry| {
        entry.stage == AuditProjectionStage::After
            && entry.action_kind == "extension_lifecycle"
            && entry.action_target.is_none()
            && entry.extension_id.as_ref().unwrap().as_str() == "echo"
            && entry.output_bytes.is_none()
    }));

    let projection_json = serde_json::to_string(&snapshot).unwrap();
    let jsonl_bytes = read_directory_text(&store_root);
    for forbidden in [
        "extension_raw_description_sentinel_3022",
        "extension_raw_asset_sentinel_3022",
        "extension_raw_schema_sentinel_3022",
    ] {
        assert!(
            !projection_json.contains(forbidden),
            "extension lifecycle projection leaked {forbidden}: {projection_json}"
        );
        assert!(
            !jsonl_bytes.contains(forbidden),
            "durable extension lifecycle audit bytes leaked {forbidden}: {jsonl_bytes}"
        );
    }
}

struct DurableExtensionLifecycleAuditSink {
    audit: Arc<dyn AuditSink>,
}

impl DurableExtensionLifecycleAuditSink {
    fn new(audit: Arc<dyn AuditSink>) -> Self {
        Self { audit }
    }
}

#[async_trait]
impl ExtensionLifecycleEventSink for DurableExtensionLifecycleAuditSink {
    async fn record_extension_lifecycle_event(
        &self,
        event: ExtensionLifecycleEvent,
    ) -> Result<(), ExtensionError> {
        self.audit
            .emit_audit(extension_lifecycle_audit(event))
            .await
            .map_err(extension_lifecycle_audit_error)
    }
}

fn extension_lifecycle_audit(event: ExtensionLifecycleEvent) -> AuditEnvelope {
    let extension_id = event.extension_id;
    AuditEnvelope {
        event_id: AuditEventId::new(),
        correlation_id: CorrelationId::new(),
        stage: AuditStage::After,
        timestamp: Utc::now(),
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("alice").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
        process_id: None,
        approval_request_id: None,
        extension_id: Some(extension_id.clone()),
        action: ActionSummary {
            kind: "extension_lifecycle".to_string(),
            target: None,
            effects: vec![EffectKind::ModifyExtension],
        },
        decision: DecisionSummary {
            kind: extension_lifecycle_decision_kind(event.operation).to_string(),
            reason: None,
            actor: None,
        },
        result: None,
    }
}

fn extension_lifecycle_decision_kind(operation: ExtensionLifecycleOperation) -> &'static str {
    match operation {
        ExtensionLifecycleOperation::Install => "extension_installed",
        ExtensionLifecycleOperation::Update => "extension_updated",
        ExtensionLifecycleOperation::Remove => "extension_removed",
        ExtensionLifecycleOperation::Enable => "extension_enabled",
        ExtensionLifecycleOperation::Disable => "extension_disabled",
    }
}

fn extension_lifecycle_audit_error(error: EventError) -> ExtensionError {
    let _ = error;
    ExtensionError::LifecycleEventSink {
        extension_id: ExtensionId::new("echo").unwrap(),
        operation: ExtensionLifecycleOperation::Install,
    }
}

fn extension_resource_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("alice").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn read_directory_text(root: &Path) -> String {
    let mut output = String::new();
    read_directory_text_into(root, &mut output);
    output
}

fn read_directory_text_into(path: &Path, output: &mut String) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            read_directory_text_into(&entry.unwrap().path(), output);
        }
    } else if path.is_file() {
        output.push_str(&fs::read_to_string(path).unwrap_or_default());
    }
}

const EXTENSION_MANIFEST_WITH_RAW_SENTINELS: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "extension_raw_description_sentinel_3022"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/extension_raw_asset_sentinel_3022.wasm"

[[capabilities]]
id = "echo.say"
description = "Echo safely"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/echo/extension_raw_schema_sentinel_3022.input.v1.json"
output_schema_ref = "schemas/echo/extension_raw_schema_sentinel_3022.output.v1.json"
"#;
