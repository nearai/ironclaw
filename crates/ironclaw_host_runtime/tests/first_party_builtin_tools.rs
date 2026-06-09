use std::{
    collections::{BTreeMap, HashMap},
    io::Write,
    net::{IpAddr, Ipv4Addr},
    path::Path,
    sync::{Arc, LazyLock},
    thread,
    time::Duration,
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
#[cfg(feature = "test-support")]
use chrono::{DateTime, Datelike, TimeZone, Utc};
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_events::InMemoryAuditSink;
use ironclaw_extensions::ExtensionRegistry;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
use ironclaw_filesystem::{InMemoryBackend, LocalFilesystem, RootFilesystem};
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, CapabilitySurfacePolicy, CapabilitySurfaceVersion,
    CommandExecutionOutput, CommandExecutionRequest, ECHO_CAPABILITY_ID, GLOB_CAPABILITY_ID,
    GREP_CAPABILITY_ID, HTTP_CAPABILITY_ID, HTTP_SAVE_CAPABILITY_ID, HostRuntime,
    HostRuntimeServices, JSON_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID, MEMORY_READ_CAPABILITY_ID,
    MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID,
    READ_FILE_CAPABILITY_ID, RuntimeCapabilityFailure, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeFailureKind, RuntimeProcessError, RuntimeProcessPort,
    SHELL_CAPABILITY_ID, SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID,
    SKILL_REMOVE_CAPABILITY_ID, SPAWN_SUBAGENT_CAPABILITY_ID, SandboxCommandTransport, SurfaceKind,
    TIME_CAPABILITY_ID, TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID,
    TRIGGER_REMOVE_CAPABILITY_ID, TenantSandboxProcessPort, ToolCallHttpEgress, TriggerCreateHook,
    VisibleCapabilityAccess, VisibleCapabilityRequest, WRITE_FILE_CAPABILITY_ID,
    builtin_first_party_handlers, builtin_first_party_handlers_with_trigger_create_hook,
    builtin_first_party_package,
};
#[cfg(feature = "test-support")]
use ironclaw_host_runtime::{
    TriggerManagementClock, builtin_first_party_handlers_with_trigger_clock,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpResponse, NetworkHttpTransport,
    NetworkResolver, NetworkTransportRequest, NetworkUsage, PolicyNetworkHttpEgress,
};
use ironclaw_resources::{InMemoryResourceGovernor, ResourceAccount};
use ironclaw_secrets::InMemorySecretStore;
use ironclaw_triggers::{
    ClaimDueFireRequest, ClearActiveFireRequest, FireAcceptedRequest, InMemoryTriggerRepository,
    MAX_TRIGGER_NAME_BYTES, MAX_TRIGGER_PROMPT_BYTES, TriggerError, TriggerRecord,
    TriggerRepository, TriggerRunHistoryStatus, TriggerRunRecord,
};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use ironclaw_turns::TurnRunId;
use serde_json::{Value, json};

#[tokio::test]
async fn builtin_first_party_package_declares_expected_capabilities() {
    let package = builtin_first_party_package().unwrap();
    assert_eq!(package.id, provider_id());
    assert_eq!(package.manifest.runtime_kind(), RuntimeKind::FirstParty);

    let ids = package
        .capabilities
        .iter()
        .map(|descriptor| descriptor.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, all_builtin_capability_ids());
    for descriptor in &package.capabilities {
        let expected_permission = match descriptor.id.as_str() {
            HTTP_CAPABILITY_ID
            | HTTP_SAVE_CAPABILITY_ID
            | SHELL_CAPABILITY_ID
            | SPAWN_SUBAGENT_CAPABILITY_ID
            | SKILL_INSTALL_CAPABILITY_ID
            | SKILL_REMOVE_CAPABILITY_ID
            | TRIGGER_CREATE_CAPABILITY_ID
            | TRIGGER_REMOVE_CAPABILITY_ID => PermissionMode::Ask,
            _ => PermissionMode::Allow,
        };
        assert_eq!(descriptor.default_permission, expected_permission);
    }

    for descriptor in package
        .capabilities
        .iter()
        .filter(|descriptor| is_coding_capability_id(descriptor.id.as_str()))
    {
        assert_coding_manifest_contract(descriptor);
    }
    let skill_install = package
        .capabilities
        .iter()
        .find(|descriptor| descriptor.id.as_str() == SKILL_INSTALL_CAPABILITY_ID)
        .expect("skill_install manifest");
    assert_eq!(
        skill_install.effects,
        vec![
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::DeleteFilesystem,
            EffectKind::Network
        ]
    );
    let http = package
        .capabilities
        .iter()
        .find(|descriptor| descriptor.id.as_str() == HTTP_CAPABILITY_ID)
        .expect("http manifest");
    assert_eq!(
        http.effects,
        vec![EffectKind::DispatchCapability, EffectKind::Network]
    );
    let http_save = package
        .capabilities
        .iter()
        .find(|descriptor| descriptor.id.as_str() == HTTP_SAVE_CAPABILITY_ID)
        .expect("http save manifest");
    assert_eq!(
        http_save.effects,
        vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::WriteFilesystem
        ]
    );

    let memory_write = package
        .capabilities
        .iter()
        .find(|descriptor| descriptor.id.as_str() == MEMORY_WRITE_CAPABILITY_ID)
        .expect("memory write manifest");
    assert_eq!(
        memory_write.effects,
        vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem]
    );
    for capability_id in [
        MEMORY_SEARCH_CAPABILITY_ID,
        MEMORY_READ_CAPABILITY_ID,
        MEMORY_TREE_CAPABILITY_ID,
    ] {
        let descriptor = package
            .capabilities
            .iter()
            .find(|descriptor| descriptor.id.as_str() == capability_id)
            .expect("memory read-like manifest");
        assert_eq!(descriptor.effects, vec![EffectKind::ReadFilesystem]);
    }

    let handlers =
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap();
    for id in all_builtin_capability_ids() {
        assert!(handlers.contains_handler(&capability_id(id)));
    }
}

fn assert_coding_manifest_contract(descriptor: &CapabilityDescriptor) {
    let expected_effects = match descriptor.id.as_str() {
        WRITE_FILE_CAPABILITY_ID => vec![EffectKind::WriteFilesystem],
        APPLY_PATCH_CAPABILITY_ID => vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
        _ => vec![EffectKind::ReadFilesystem],
    };
    assert_eq!(descriptor.effects, expected_effects);
    assert_eq!(descriptor.default_permission, PermissionMode::Allow);
}

fn is_coding_capability_id(id: &str) -> bool {
    matches!(
        id,
        READ_FILE_CAPABILITY_ID
            | WRITE_FILE_CAPABILITY_ID
            | LIST_DIR_CAPABILITY_ID
            | GLOB_CAPABILITY_ID
            | GREP_CAPABILITY_ID
            | APPLY_PATCH_CAPABILITY_ID
    )
}

#[tokio::test]
async fn builtin_first_party_package_omits_prompt_doc_refs() {
    let package = builtin_first_party_package().unwrap();

    assert!(
        package
            .manifest
            .capabilities
            .iter()
            .all(|capability| capability.prompt_doc_ref.is_none())
    );
}

#[tokio::test]
async fn builtin_first_party_surface_lists_allowed_tools_in_registry_order() {
    let runtime = runtime();
    let request = VisibleCapabilityRequest::new(
        execution_context(all_builtin_capability_ids()),
        SurfaceKind::new("agent_loop").unwrap(),
    )
    .with_policy(CapabilitySurfacePolicy::allow_all())
    .with_provider_trust(provider_trust());

    let surface = runtime.visible_capabilities(request).await.unwrap();

    let ids = surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, all_builtin_capability_ids());
    assert!(
        surface
            .capabilities
            .iter()
            .all(|capability| capability.access == VisibleCapabilityAccess::Available)
    );
    assert!(
        surface
            .capabilities
            .iter()
            .all(|capability| capability.estimated_resources.output_bytes.is_some())
    );
    let shell = surface
        .capabilities
        .iter()
        .find(|capability| capability.descriptor.id.as_str() == SHELL_CAPABILITY_ID)
        .expect("shell capability must be visible");
    assert_eq!(shell.estimated_resources.process_count, Some(1));

    let spawn = surface
        .capabilities
        .iter()
        .find(|capability| capability.descriptor.id.as_str() == SPAWN_SUBAGENT_CAPABILITY_ID)
        .expect("spawn_subagent capability must be visible");
    let properties = spawn
        .descriptor
        .parameters_schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("spawn_subagent schema properties");
    assert!(properties.contains_key("subagent_type"));
    assert!(properties.contains_key("task"));
    assert!(properties.contains_key("handoff"));
    assert!(!properties.contains_key("mode"));
    assert!(!properties.contains_key("run_in_background"));
}

#[tokio::test]
async fn builtin_first_party_surface_hides_runtime_policy_impossible_tools() {
    let runtime = runtime_with_policy(network_denied_policy());
    let request = VisibleCapabilityRequest::new(
        execution_context(all_builtin_capability_ids()),
        SurfaceKind::new("agent_loop").unwrap(),
    )
    .with_policy(CapabilitySurfacePolicy::allow_all())
    .with_provider_trust(provider_trust());

    let surface = runtime.visible_capabilities(request).await.unwrap();

    let ids = surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.as_str())
        .collect::<Vec<_>>();
    assert!(!ids.contains(&HTTP_CAPABILITY_ID));
    assert!(!ids.contains(&HTTP_SAVE_CAPABILITY_ID));
    assert!(ids.contains(&ECHO_CAPABILITY_ID));
}

#[tokio::test]
async fn builtin_trigger_create_stamps_caller_scope_and_persists_record() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID]);

    let output = invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Daily summary",
            "prompt": "Summarize yesterday",
            "cron": "0 8 * * *"
        }),
        context.clone(),
    )
    .await
    .unwrap();

    let trigger = &output["trigger"];
    assert_eq!(trigger["name"], json!("Daily summary"));
    assert!(trigger.get("prompt").is_none());
    assert_eq!(trigger["source"], json!("schedule"));
    assert_eq!(trigger["completion_policy"], json!("recurring"));
    assert_eq!(trigger["state"], json!("scheduled"));
    assert!(trigger.get("tenant_id").is_none());
    assert!(trigger.get("creator_user_id").is_none());
    assert_eq!(trigger["agent_id"], json!("default"));
    assert_eq!(trigger["project_id"], json!("bootstrap"));
    assert_eq!(trigger["last_status"], Value::Null);
    assert_eq!(trigger["is_active"], json!(false));
    assert!(trigger.get("last_fired_slot").is_none());
    assert!(trigger.get("active_fire_slot").is_none());
    assert!(trigger.get("active_run_ref").is_none());

    let records = repository
        .list_triggers(context.resource_scope.tenant_id.clone())
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].prompt, "Summarize yesterday");
    assert_eq!(records[0].creator_user_id, context.resource_scope.user_id);
    assert_eq!(records[0].agent_id, context.resource_scope.agent_id);
    assert_eq!(records[0].project_id, context.resource_scope.project_id);
}

#[tokio::test]
async fn builtin_trigger_create_runs_create_hook_after_persistence() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let hook = Arc::new(PersistedRecordTriggerCreateHook::new(repository.clone()));
    let runtime = runtime_with_trigger_repository_and_create_hook(repository.clone(), hook.clone());
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID]);

    invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Hooked trigger",
            "prompt": "Pair trigger creator",
            "cron": "0 8 * * *"
        }),
        context.clone(),
    )
    .await
    .unwrap();

    let hooked_records = hook.records();
    assert_eq!(hooked_records.len(), 1);
    assert_eq!(hooked_records[0].name, "Hooked trigger");
    assert_eq!(hooked_records[0].prompt, "Pair trigger creator");
    assert_eq!(
        hooked_records[0].creator_user_id,
        context.resource_scope.user_id
    );
    assert_eq!(hooked_records[0].agent_id, context.resource_scope.agent_id);
    assert_eq!(
        hooked_records[0].project_id,
        context.resource_scope.project_id
    );

    let records = repository
        .list_triggers(context.resource_scope.tenant_id)
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].trigger_id, hooked_records[0].trigger_id);
}

#[tokio::test]
async fn builtin_trigger_create_maps_create_hook_error_to_backend_and_rolls_back_record() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository_and_create_hook(
        repository.clone(),
        Arc::new(FailingTriggerCreateHook),
    );
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID]);

    let error = invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Hook failure",
            "prompt": "Do not persist this trigger",
            "cron": "0 8 * * *"
        }),
        context.clone(),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Backend);
    assert!(
        repository
            .list_triggers(context.resource_scope.tenant_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn builtin_trigger_create_surfaces_rollback_error_when_cleanup_fails() {
    let repository = Arc::new(RemoveFailingTriggerRepository::default());
    let runtime = runtime_with_trigger_repository_and_create_hook(
        repository.clone(),
        Arc::new(FailingTriggerCreateHook),
    );
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID]);

    let failure = invoke_failure_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Rollback failure",
            "prompt": "Surface the rollback failure as the user-visible cause",
            "cron": "0 8 * * *"
        }),
        context.clone(),
    )
    .await;

    assert_eq!(failure.kind, RuntimeFailureKind::Backend);
    assert_eq!(
        failure.safe_summary().as_deref(),
        Some("trigger create rollback failed after hook error")
    );
    assert_eq!(repository.remove_attempts(), 1);
    assert_eq!(
        repository
            .list_triggers(context.resource_scope.tenant_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn builtin_trigger_create_rejects_sub_minute_schedule_before_persistence() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID]);

    let error = invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Too fast",
            "prompt": "Run constantly",
            "cron": "* * * * * *"
        }),
        context.clone(),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(
        repository
            .list_triggers(context.resource_scope.tenant_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[cfg(feature = "test-support")]
#[tokio::test]
async fn builtin_trigger_create_rejects_schedule_with_no_future_slot_before_persistence() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let future_year = Utc::now().year() + 1;
    let after_schedule_expires = Utc
        .with_ymd_and_hms(future_year + 1, 1, 1, 0, 0, 0)
        .unwrap();
    let runtime = runtime_with_trigger_repository_and_clock(
        repository.clone(),
        Arc::new(FixedTriggerClock(after_schedule_expires)),
    );
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID]);

    let error = invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Expired finite schedule",
            "prompt": "Run once in the finite year",
            "cron": format!("0 0 8 * * * {future_year}")
        }),
        context.clone(),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(
        repository
            .list_triggers(context.resource_scope.tenant_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn builtin_trigger_create_rejects_malformed_input_before_persistence() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID]);

    let error = invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Missing prompt",
            "cron": "0 8 * * *"
        }),
        context.clone(),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(
        repository
            .list_triggers(context.resource_scope.tenant_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn builtin_trigger_create_rejects_blank_name_or_prompt_before_persistence() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID]);

    for input in [
        json!({
            "name": " ",
            "prompt": "Run work",
            "cron": "0 8 * * *"
        }),
        json!({
            "name": "Blank prompt",
            "prompt": " ",
            "cron": "0 8 * * *"
        }),
    ] {
        let error = invoke_with_context(
            &runtime,
            TRIGGER_CREATE_CAPABILITY_ID,
            input,
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::InvalidInput);
    }

    assert!(
        repository
            .list_triggers(context.resource_scope.tenant_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn builtin_trigger_create_rejects_oversized_name_or_prompt_before_persistence() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID]);

    for input in [
        json!({
            "name": "x".repeat(MAX_TRIGGER_NAME_BYTES + 1),
            "prompt": "Run work",
            "cron": "0 8 * * *"
        }),
        json!({
            "name": "Oversized prompt",
            "prompt": "x".repeat(MAX_TRIGGER_PROMPT_BYTES + 1),
            "cron": "0 8 * * *"
        }),
    ] {
        let error = invoke_with_context(
            &runtime,
            TRIGGER_CREATE_CAPABILITY_ID,
            input,
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::InvalidInput);
    }

    assert!(
        repository
            .list_triggers(context.resource_scope.tenant_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn builtin_trigger_create_applies_first_party_input_size_bound() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID]);

    let error = invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Large ignored field",
            "prompt": "Run work",
            "cron": "0 8 * * *",
            "padding": "x".repeat(1_048_576)
        }),
        context.clone(),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Resource);
    assert!(
        repository
            .list_triggers(context.resource_scope.tenant_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn builtin_trigger_list_and_remove_are_caller_scoped() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let owner_context = execution_context([
        TRIGGER_CREATE_CAPABILITY_ID,
        TRIGGER_LIST_CAPABILITY_ID,
        TRIGGER_REMOVE_CAPABILITY_ID,
    ]);
    let mut foreign_context =
        execution_context([TRIGGER_LIST_CAPABILITY_ID, TRIGGER_REMOVE_CAPABILITY_ID]);
    foreign_context.user_id = UserId::new("other-user").unwrap();
    foreign_context.resource_scope.user_id = foreign_context.user_id.clone();

    let created = invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Owned trigger",
            "prompt": "Run owned work",
            "cron": "0 8 * * *"
        }),
        owner_context.clone(),
    )
    .await
    .unwrap();
    assert!(created["trigger"].get("prompt").is_none());
    let trigger_id = created["trigger"]["trigger_id"].as_str().unwrap();

    let foreign_list = invoke_with_context(
        &runtime,
        TRIGGER_LIST_CAPABILITY_ID,
        json!({}),
        foreign_context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(foreign_list["triggers"], json!([]));

    let foreign_remove = invoke_with_context(
        &runtime,
        TRIGGER_REMOVE_CAPABILITY_ID,
        json!({ "trigger_id": trigger_id }),
        foreign_context,
    )
    .await
    .unwrap();
    assert_eq!(foreign_remove["removed"], json!(false));

    let owner_list = invoke_with_context(
        &runtime,
        TRIGGER_LIST_CAPABILITY_ID,
        json!({}),
        owner_context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(owner_list["triggers"].as_array().unwrap().len(), 1);
    assert!(owner_list["triggers"][0].get("last_status").is_some());
    assert_eq!(owner_list["triggers"][0]["is_active"], json!(false));
    assert!(owner_list["triggers"][0].get("prompt").is_none());
    assert!(owner_list["triggers"][0].get("tenant_id").is_none());
    assert!(owner_list["triggers"][0].get("creator_user_id").is_none());
    assert!(owner_list["triggers"][0].get("last_fired_slot").is_none());
    assert!(owner_list["triggers"][0].get("active_fire_slot").is_none());
    assert!(owner_list["triggers"][0].get("active_run_ref").is_none());

    let owner_remove = invoke_with_context(
        &runtime,
        TRIGGER_REMOVE_CAPABILITY_ID,
        json!({ "trigger_id": trigger_id }),
        owner_context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(owner_remove["removed"], json!(true));
    assert_eq!(owner_remove["trigger"]["trigger_id"], json!(trigger_id));
    assert_eq!(owner_remove["trigger"]["name"], json!("Owned trigger"));
    assert!(owner_remove["trigger"].get("prompt").is_none());
    assert!(owner_remove["trigger"].get("tenant_id").is_none());
    assert!(owner_remove["trigger"].get("creator_user_id").is_none());
    assert!(owner_remove["trigger"].get("active_fire_slot").is_none());
    assert!(owner_remove["trigger"].get("active_run_ref").is_none());

    let records = repository
        .list_triggers(owner_context.resource_scope.tenant_id)
        .await
        .unwrap();
    assert!(records.is_empty());
}

#[tokio::test]
async fn builtin_trigger_list_shows_active_state_without_run_identifiers() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID]);

    let created = invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Active trigger",
            "prompt": "Run active work",
            "cron": "0 8 * * *"
        }),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(created["trigger"]["is_active"], json!(false));

    let mut records = repository
        .list_triggers(context.resource_scope.tenant_id.clone())
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    records[0].active_fire_slot = Some(records[0].next_run_at);
    records[0].active_run_ref =
        Some(TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").unwrap());
    repository.upsert_trigger(records.remove(0)).await.unwrap();

    let listed = invoke_with_context(&runtime, TRIGGER_LIST_CAPABILITY_ID, json!({}), context)
        .await
        .unwrap();
    let trigger = &listed["triggers"][0];
    assert_eq!(trigger["is_active"], json!(true));
    assert!(trigger.get("active_fire_slot").is_none());
    assert!(trigger.get("active_run_ref").is_none());
}

#[tokio::test]
async fn builtin_trigger_create_list_and_remove_use_full_request_scope() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let mut owner_context = execution_context([
        TRIGGER_CREATE_CAPABILITY_ID,
        TRIGGER_LIST_CAPABILITY_ID,
        TRIGGER_REMOVE_CAPABILITY_ID,
    ]);
    set_context_scope(
        &mut owner_context,
        TenantId::new("scoped-tenant").unwrap(),
        UserId::new("scoped-user").unwrap(),
        Some(AgentId::new("scoped-agent").unwrap()),
        Some(ProjectId::new("scoped-project").unwrap()),
    );

    let mut other_agent_context = owner_context.clone();
    other_agent_context.agent_id = Some(AgentId::new("other-agent").unwrap());
    other_agent_context.resource_scope.agent_id = other_agent_context.agent_id.clone();

    let mut other_project_context = owner_context.clone();
    other_project_context.project_id = Some(ProjectId::new("other-project").unwrap());
    other_project_context.resource_scope.project_id = other_project_context.project_id.clone();

    let mut other_tenant_context = owner_context.clone();
    other_tenant_context.tenant_id = TenantId::new("other-tenant").unwrap();
    other_tenant_context.resource_scope.tenant_id = other_tenant_context.tenant_id.clone();

    let created = invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Scoped trigger",
            "prompt": "Run scoped work",
            "cron": "0 8 * * *"
        }),
        owner_context.clone(),
    )
    .await
    .unwrap();
    let trigger = &created["trigger"];
    assert!(trigger.get("tenant_id").is_none());
    assert!(trigger.get("creator_user_id").is_none());
    assert_eq!(trigger["agent_id"], json!("scoped-agent"));
    assert_eq!(trigger["project_id"], json!("scoped-project"));
    assert_eq!(trigger["is_active"], json!(false));
    assert!(trigger.get("prompt").is_none());
    assert!(trigger.get("last_fired_slot").is_none());
    assert!(trigger.get("active_fire_slot").is_none());
    assert!(trigger.get("active_run_ref").is_none());
    let trigger_id = trigger["trigger_id"].as_str().unwrap();

    let other_agent_list = invoke_with_context(
        &runtime,
        TRIGGER_LIST_CAPABILITY_ID,
        json!({}),
        other_agent_context,
    )
    .await
    .unwrap();
    assert_eq!(other_agent_list["triggers"], json!([]));

    let other_project_remove = invoke_with_context(
        &runtime,
        TRIGGER_REMOVE_CAPABILITY_ID,
        json!({ "trigger_id": trigger_id }),
        other_project_context,
    )
    .await
    .unwrap();
    assert_eq!(other_project_remove["removed"], json!(false));

    let other_tenant_remove = invoke_with_context(
        &runtime,
        TRIGGER_REMOVE_CAPABILITY_ID,
        json!({ "trigger_id": trigger_id }),
        other_tenant_context,
    )
    .await
    .unwrap();
    assert_eq!(other_tenant_remove["removed"], json!(false));

    let owner_remove = invoke_with_context(
        &runtime,
        TRIGGER_REMOVE_CAPABILITY_ID,
        json!({ "trigger_id": trigger_id }),
        owner_context,
    )
    .await
    .unwrap();
    assert_eq!(owner_remove["removed"], json!(true));
    assert_eq!(owner_remove["trigger"]["trigger_id"], json!(trigger_id));
    assert_eq!(owner_remove["trigger"]["name"], json!("Scoped trigger"));
    assert!(owner_remove["trigger"].get("prompt").is_none());
    assert!(owner_remove["trigger"].get("agent_id").is_none());
    assert!(owner_remove["trigger"].get("project_id").is_none());
    assert!(owner_remove["trigger"].get("active_fire_slot").is_none());
    assert!(owner_remove["trigger"].get("active_run_ref").is_none());
}

#[tokio::test]
async fn builtin_trigger_create_round_trips_nullable_agent_and_project_scope() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository);
    let mut context = execution_context([TRIGGER_CREATE_CAPABILITY_ID]);
    context.agent_id = None;
    context.project_id = None;
    context.resource_scope.agent_id = None;
    context.resource_scope.project_id = None;

    let created = invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Unscoped trigger",
            "prompt": "Run unscoped work",
            "cron": "0 8 * * *"
        }),
        context,
    )
    .await
    .unwrap();

    assert_eq!(created["trigger"]["agent_id"], Value::Null);
    assert_eq!(created["trigger"]["project_id"], Value::Null);
    assert_eq!(created["trigger"]["is_active"], json!(false));
}

#[tokio::test]
async fn builtin_trigger_list_applies_user_surface_limit_boundaries() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository);
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID]);

    for index in 0..101 {
        invoke_with_context(
            &runtime,
            TRIGGER_CREATE_CAPABILITY_ID,
            json!({
                "name": format!("Trigger {index}"),
                "prompt": "Run work",
                "cron": "0 8 * * *"
            }),
            context.clone(),
        )
        .await
        .unwrap();
    }

    let empty = invoke_with_context(
        &runtime,
        TRIGGER_LIST_CAPABILITY_ID,
        json!({ "limit": 0 }),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(empty["triggers"], json!([]));

    let listed = invoke_with_context(
        &runtime,
        TRIGGER_LIST_CAPABILITY_ID,
        json!({ "limit": 2 }),
        context.clone(),
    )
    .await
    .unwrap();

    assert_eq!(listed["triggers"].as_array().unwrap().len(), 2);

    let defaulted = invoke_with_context(
        &runtime,
        TRIGGER_LIST_CAPABILITY_ID,
        json!({}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(defaulted["triggers"].as_array().unwrap().len(), 100);

    let clamped = invoke_with_context(
        &runtime,
        TRIGGER_LIST_CAPABILITY_ID,
        json!({ "limit": 200 }),
        context,
    )
    .await
    .unwrap();
    assert_eq!(clamped["triggers"].as_array().unwrap().len(), 100);
}

#[tokio::test]
async fn builtin_trigger_list_embeds_recent_run_history_with_run_limit() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID]);

    invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Historical trigger",
            "prompt": "Create history rows",
            "cron": "0 8 * * *"
        }),
        context.clone(),
    )
    .await
    .unwrap();

    let record = repository
        .list_triggers(context.resource_scope.tenant_id.clone())
        .await
        .unwrap()
        .pop()
        .expect("persisted trigger");
    let first_fire_slot = record.next_run_at;
    let first_run_id = TurnRunId::new();
    repository
        .claim_due_fire(ClaimDueFireRequest {
            tenant_id: record.tenant_id.clone(),
            trigger_id: record.trigger_id,
            fire_slot: first_fire_slot,
            now: first_fire_slot,
        })
        .await
        .unwrap();
    repository
        .mark_fire_accepted(FireAcceptedRequest {
            tenant_id: record.tenant_id.clone(),
            trigger_id: record.trigger_id,
            fire_slot: first_fire_slot,
            run_id: first_run_id,
            submitted_at: first_fire_slot + chrono::Duration::seconds(1),
            next_run_at: first_fire_slot + chrono::Duration::minutes(1),
        })
        .await
        .unwrap();
    repository
        .clear_active_fire(ClearActiveFireRequest {
            tenant_id: record.tenant_id.clone(),
            trigger_id: record.trigger_id,
            fire_slot: first_fire_slot,
            run_id: first_run_id,
            status: TriggerRunHistoryStatus::Ok,
        })
        .await
        .unwrap();

    let second_fire_slot = first_fire_slot + chrono::Duration::minutes(1);
    let second_run_id = TurnRunId::new();
    repository
        .claim_due_fire(ClaimDueFireRequest {
            tenant_id: record.tenant_id.clone(),
            trigger_id: record.trigger_id,
            fire_slot: second_fire_slot,
            now: second_fire_slot,
        })
        .await
        .unwrap();
    repository
        .mark_fire_accepted(FireAcceptedRequest {
            tenant_id: record.tenant_id.clone(),
            trigger_id: record.trigger_id,
            fire_slot: second_fire_slot,
            run_id: second_run_id,
            submitted_at: second_fire_slot + chrono::Duration::seconds(1),
            next_run_at: second_fire_slot + chrono::Duration::minutes(1),
        })
        .await
        .unwrap();
    repository
        .clear_active_fire(ClearActiveFireRequest {
            tenant_id: record.tenant_id.clone(),
            trigger_id: record.trigger_id,
            fire_slot: second_fire_slot,
            run_id: second_run_id,
            status: TriggerRunHistoryStatus::Error,
        })
        .await
        .unwrap();

    let third_fire_slot = second_fire_slot + chrono::Duration::minutes(1);
    let third_run_id = TurnRunId::new();
    repository
        .claim_due_fire(ClaimDueFireRequest {
            tenant_id: record.tenant_id.clone(),
            trigger_id: record.trigger_id,
            fire_slot: third_fire_slot,
            now: third_fire_slot,
        })
        .await
        .unwrap();
    repository
        .mark_fire_accepted(FireAcceptedRequest {
            tenant_id: record.tenant_id,
            trigger_id: record.trigger_id,
            fire_slot: third_fire_slot,
            run_id: third_run_id,
            submitted_at: third_fire_slot + chrono::Duration::seconds(1),
            next_run_at: third_fire_slot + chrono::Duration::minutes(1),
        })
        .await
        .unwrap();

    let listed = invoke_with_context(
        &runtime,
        TRIGGER_LIST_CAPABILITY_ID,
        json!({ "run_limit": 3 }),
        context,
    )
    .await
    .unwrap();

    let runs = listed["triggers"][0]["recent_runs"].as_array().unwrap();
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[0]["run_id"], json!(third_run_id.to_string()));
    assert_eq!(runs[0]["status"], json!("running"));
    assert_eq!(runs[0]["completed_at"], Value::Null);
    assert_eq!(runs[1]["run_id"], json!(second_run_id.to_string()));
    assert_eq!(runs[1]["status"], json!("error"));
    assert_ne!(runs[1]["completed_at"], Value::Null);
    assert_eq!(runs[2]["run_id"], json!(first_run_id.to_string()));
    assert_eq!(runs[2]["status"], json!("ok"));
    assert_ne!(runs[2]["completed_at"], Value::Null);
    assert_eq!(
        runs[0]["thread_id"].as_str().unwrap().len(),
        64,
        "trigger route thread ids are deterministic hex identifiers"
    );
}

#[tokio::test]
async fn builtin_trigger_list_with_zero_run_limit_returns_empty_recent_runs() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID]);

    invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Zero run limit trigger",
            "prompt": "Create history rows",
            "cron": "0 8 * * *"
        }),
        context.clone(),
    )
    .await
    .unwrap();

    let record = repository
        .list_triggers(context.resource_scope.tenant_id.clone())
        .await
        .unwrap()
        .pop()
        .expect("persisted trigger");
    seed_completed_trigger_runs(&repository, &record, 1).await;

    let listed = invoke_with_context(
        &runtime,
        TRIGGER_LIST_CAPABILITY_ID,
        json!({ "run_limit": 0 }),
        context,
    )
    .await
    .unwrap();

    assert_eq!(listed["triggers"][0]["recent_runs"], json!([]));
}

#[tokio::test]
async fn builtin_trigger_list_clamps_oversized_run_limit_to_max() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID]);

    invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Oversized run limit trigger",
            "prompt": "Create many history rows",
            "cron": "0 8 * * *"
        }),
        context.clone(),
    )
    .await
    .unwrap();

    let record = repository
        .list_triggers(context.resource_scope.tenant_id.clone())
        .await
        .unwrap()
        .pop()
        .expect("persisted trigger");
    seed_completed_trigger_runs(&repository, &record, 101).await;

    let listed = invoke_with_context(
        &runtime,
        TRIGGER_LIST_CAPABILITY_ID,
        json!({ "run_limit": 200 }),
        context,
    )
    .await
    .unwrap();

    assert_eq!(
        listed["triggers"][0]["recent_runs"]
            .as_array()
            .unwrap()
            .len(),
        100
    );
}

async fn seed_completed_trigger_runs(
    repository: &InMemoryTriggerRepository,
    record: &TriggerRecord,
    count: usize,
) {
    for index in 0..count {
        let fire_slot = record.next_run_at + chrono::Duration::minutes(index as i64);
        let run_id = TurnRunId::new();
        repository
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: record.tenant_id.clone(),
                trigger_id: record.trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .unwrap();
        repository
            .mark_fire_accepted(FireAcceptedRequest {
                tenant_id: record.tenant_id.clone(),
                trigger_id: record.trigger_id,
                fire_slot,
                run_id,
                submitted_at: fire_slot + chrono::Duration::seconds(1),
                next_run_at: fire_slot + chrono::Duration::minutes(1),
            })
            .await
            .unwrap();
        repository
            .clear_active_fire(ClearActiveFireRequest {
                tenant_id: record.tenant_id.clone(),
                trigger_id: record.trigger_id,
                fire_slot,
                run_id,
                status: TriggerRunHistoryStatus::Ok,
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn builtin_trigger_remove_rejects_invalid_trigger_id() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository);
    let context = execution_context([TRIGGER_REMOVE_CAPABILITY_ID]);

    let error = invoke_with_context(
        &runtime,
        TRIGGER_REMOVE_CAPABILITY_ID,
        json!({ "trigger_id": "not a trigger id" }),
        context,
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn builtin_trigger_list_rejects_non_integer_limit() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let context = execution_context([TRIGGER_LIST_CAPABILITY_ID]);

    let error = invoke_with_context(
        &runtime,
        TRIGGER_LIST_CAPABILITY_ID,
        json!({ "limit": "many" }),
        context.clone(),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(
        repository
            .list_triggers(context.resource_scope.tenant_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn builtin_trigger_list_rejects_non_integer_run_limit() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository.clone());
    let context = execution_context([TRIGGER_LIST_CAPABILITY_ID]);

    let error = invoke_with_context(
        &runtime,
        TRIGGER_LIST_CAPABILITY_ID,
        json!({ "run_limit": "many" }),
        context.clone(),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(
        repository
            .list_triggers(context.resource_scope.tenant_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn builtin_trigger_remove_rejects_malformed_input() {
    let repository = Arc::new(InMemoryTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository);
    let context = execution_context([TRIGGER_REMOVE_CAPABILITY_ID]);

    for input in [json!({}), json!({ "trigger_id": 123 })] {
        let error = invoke_with_context(
            &runtime,
            TRIGGER_REMOVE_CAPABILITY_ID,
            input,
            context.clone(),
        )
        .await
        .unwrap_err();

        assert_eq!(error, RuntimeFailureKind::InvalidInput);
    }
}

#[tokio::test]
async fn builtin_trigger_management_maps_repository_errors_to_backend() {
    let runtime = runtime_with_trigger_repository(Arc::new(FailingTriggerRepository));
    let context = execution_context([
        TRIGGER_CREATE_CAPABILITY_ID,
        TRIGGER_LIST_CAPABILITY_ID,
        TRIGGER_REMOVE_CAPABILITY_ID,
    ]);

    let create_error = invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Backend create",
            "prompt": "Run work",
            "cron": "0 8 * * *"
        }),
        context.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(create_error, RuntimeFailureKind::Backend);

    let list_error = invoke_with_context(
        &runtime,
        TRIGGER_LIST_CAPABILITY_ID,
        json!({}),
        context.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(list_error, RuntimeFailureKind::Backend);

    let remove_error = invoke_with_context(
        &runtime,
        TRIGGER_REMOVE_CAPABILITY_ID,
        json!({ "trigger_id": "01HZZZZZZZZZZZZZZZZZZZZZZZ" }),
        context,
    )
    .await
    .unwrap_err();
    assert_eq!(remove_error, RuntimeFailureKind::Backend);
}

#[tokio::test]
async fn builtin_trigger_list_maps_batch_run_history_repository_error_to_backend() {
    let repository = Arc::new(BatchRunHistoryFailingTriggerRepository::default());
    let runtime = runtime_with_trigger_repository(repository);
    let context = execution_context([TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID]);

    invoke_with_context(
        &runtime,
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Batch history failure",
            "prompt": "Create trigger before listing history",
            "cron": "0 8 * * *"
        }),
        context.clone(),
    )
    .await
    .unwrap();

    let error = invoke_with_context(&runtime, TRIGGER_LIST_CAPABILITY_ID, json!({}), context)
        .await
        .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Backend);
}

#[tokio::test]
async fn builtin_rejects_oversized_inputs_before_dispatch() {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context([JSON_CAPABILITY_ID]),
            capability_id(JSON_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({"operation": "validate", "data": "x".repeat(1_048_577)}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected resource failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Resource);
}

#[tokio::test]
async fn builtin_rejects_oversized_outputs_before_return() {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context([JSON_CAPABILITY_ID]),
            capability_id(JSON_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({"operation": "stringify", "data": {"items": vec!["xxxxxxxx"; 80_000]}}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected output-too-large failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::OutputTooLarge);
}

#[tokio::test]
async fn memory_capabilities_write_read_tree_and_search_native_reborn_memory() {
    let runtime = runtime_with_filesystem(InMemoryBackend::new());
    let context = execution_context_with_mounts(
        all_builtin_capability_ids(),
        memory_mounts(MountPermissions::read_write_list_delete()),
    );

    let write = invoke_with_context(
        &runtime,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({
            "target": "projects/alpha/notes.md",
            "content": "Architecture note: reborn memory capability search marker.",
            "append": false
        }),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(write["status"], json!("written"));
    assert_eq!(write["path"], json!("projects/alpha/notes.md"));

    let read = invoke_with_context(
        &runtime,
        MEMORY_READ_CAPABILITY_ID,
        json!({"path": "projects/alpha/notes.md"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(read["path"], json!("projects/alpha/notes.md"));
    assert!(
        read["content"]
            .as_str()
            .unwrap()
            .contains("reborn memory capability search marker")
    );

    let tree = invoke_with_context(
        &runtime,
        MEMORY_TREE_CAPABILITY_ID,
        json!({"path": "", "depth": 3}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        tree,
        json!([{"projects/": [{"alpha/": ["notes.md"]}]}]),
        "tree should preserve dynamic directory names: {tree}"
    );
    assert!(
        tree.to_string().contains("alpha/"),
        "tree should include project directory: {tree}"
    );

    let search = invoke_with_context(
        &runtime,
        MEMORY_SEARCH_CAPABILITY_ID,
        json!({"query": "capability search marker", "limit": 5}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(search["result_count"], json!(1));
    assert_eq!(
        search["results"][0]["path"],
        json!("projects/alpha/notes.md")
    );
}

#[tokio::test]
async fn memory_search_accepts_common_query_aliases_from_model_tool_calls() {
    let runtime = runtime_with_filesystem(InMemoryBackend::new());
    let context = execution_context_with_mounts(
        [MEMORY_WRITE_CAPABILITY_ID, MEMORY_SEARCH_CAPABILITY_ID],
        memory_mounts(MountPermissions::read_write_list_delete()),
    );

    invoke_with_context(
        &runtime,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({
            "target": "projects/alpha/search-alias.md",
            "content": "Search alias marker from model tool call.",
            "append": false
        }),
        context.clone(),
    )
    .await
    .unwrap();

    for input in [
        json!({"q": "alias marker", "limit": 5}),
        json!({"text": "alias marker", "limit": 5}),
        json!({"pattern": "alias marker", "limit": 5}),
    ] {
        let search = invoke_with_context(
            &runtime,
            MEMORY_SEARCH_CAPABILITY_ID,
            input,
            context.clone(),
        )
        .await
        .unwrap();
        assert_eq!(search["result_count"], json!(1));
        assert_eq!(
            search["results"][0]["path"],
            json!("projects/alpha/search-alias.md")
        );
    }
}

#[tokio::test]
async fn memory_write_metadata_overlay_can_skip_search_indexing() {
    let runtime = runtime_with_filesystem(InMemoryBackend::new());
    let context = execution_context_with_mounts(
        [MEMORY_WRITE_CAPABILITY_ID, MEMORY_SEARCH_CAPABILITY_ID],
        memory_mounts(MountPermissions::read_write_list_delete()),
    );

    invoke_with_context(
        &runtime,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({
            "target": "projects/alpha/skip-index.md",
            "content": "metadata overlay search marker",
            "append": false,
            "metadata": {"skip_indexing": true}
        }),
        context.clone(),
    )
    .await
    .unwrap();

    let search = invoke_with_context(
        &runtime,
        MEMORY_SEARCH_CAPABILITY_ID,
        json!({"query": "metadata overlay search marker", "limit": 5}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(search["result_count"], json!(0));
}

#[tokio::test]
async fn memory_write_patches_existing_document_and_rejects_missing_old_string() {
    let runtime = runtime_with_filesystem(InMemoryBackend::new());
    let context = execution_context_with_mounts(
        [MEMORY_WRITE_CAPABILITY_ID, MEMORY_READ_CAPABILITY_ID],
        memory_mounts(MountPermissions::read_write_list_delete()),
    );

    invoke_with_context(
        &runtime,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({
            "target": "projects/alpha/patch.md",
            "content": "alpha beta beta",
            "append": false
        }),
        context.clone(),
    )
    .await
    .unwrap();
    let patch = invoke_with_context(
        &runtime,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({
            "target": "projects/alpha/patch.md",
            "old_string": "beta",
            "new_string": "gamma"
        }),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(patch["status"], json!("patched"));
    assert_eq!(patch["replacements"], json!(1));

    let patch_all = invoke_with_context(
        &runtime,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({
            "target": "projects/alpha/patch.md",
            "old_string": "beta",
            "new_string": "delta",
            "replace_all": true
        }),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(patch_all["replacements"], json!(1));
    let read = invoke_with_context(
        &runtime,
        MEMORY_READ_CAPABILITY_ID,
        json!({"path": "projects/alpha/patch.md"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(read["content"], json!("alpha gamma delta"));

    let failure = invoke_with_context(
        &runtime,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({
            "target": "projects/alpha/patch.md",
            "old_string": "missing",
            "new_string": "value"
        }),
        context,
    )
    .await
    .unwrap_err();
    assert_eq!(failure, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn memory_write_bootstrap_target_clears_bootstrap_document() {
    let runtime = runtime_with_filesystem(InMemoryBackend::new());
    let context = execution_context_with_mounts(
        [MEMORY_WRITE_CAPABILITY_ID, MEMORY_READ_CAPABILITY_ID],
        memory_mounts(MountPermissions::read_write_list_delete()),
    );

    let clear = invoke_with_context(
        &runtime,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({"target": "bootstrap", "content": "ignored"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(clear["status"], json!("cleared"));

    let read = invoke_with_context(
        &runtime,
        MEMORY_READ_CAPABILITY_ID,
        json!({"path": "BOOTSTRAP.md"}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(read["content"], json!(""));
}

#[tokio::test]
async fn memory_write_daily_log_rejects_invalid_timezone() {
    let runtime = runtime_with_filesystem(InMemoryBackend::new());
    let failure = invoke_with_context(
        &runtime,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({
            "target": "daily_log",
            "content": "timezone should reject",
            "timezone": "not/a-zone"
        }),
        execution_context_with_mounts(
            [MEMORY_WRITE_CAPABILITY_ID],
            memory_mounts(MountPermissions::read_write_list_delete()),
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(failure, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn memory_write_rejects_local_filesystem_paths() {
    let runtime = runtime_with_filesystem(InMemoryBackend::new());
    let context = execution_context_with_mounts(
        [MEMORY_WRITE_CAPABILITY_ID],
        memory_mounts(MountPermissions::read_write_list_delete()),
    );

    for target in ["/Users/example/notes.md", "C:/Users/example/notes.md"] {
        let failure = invoke_with_context(
            &runtime,
            MEMORY_WRITE_CAPABILITY_ID,
            json!({
                "target": target,
                "content": "should not write"
            }),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(failure, RuntimeFailureKind::InvalidInput);
    }
}

#[tokio::test]
async fn memory_write_rejects_traversal_paths() {
    let runtime = runtime_with_filesystem(InMemoryBackend::new());
    let context = execution_context_with_mounts(
        [MEMORY_WRITE_CAPABILITY_ID],
        memory_mounts(MountPermissions::read_write_list_delete()),
    );

    for target in ["daily/../SECRET.md", r"daily\..\SECRET.md"] {
        let failure = invoke_with_context(
            &runtime,
            MEMORY_WRITE_CAPABILITY_ID,
            json!({
                "target": target,
                "content": "should not write"
            }),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(failure, RuntimeFailureKind::InvalidInput);
    }
}

#[tokio::test]
async fn memory_write_rejects_non_string_target() {
    let runtime = runtime_with_filesystem(InMemoryBackend::new());
    for target in [json!(42), json!(true)] {
        let failure = invoke_with_context(
            &runtime,
            MEMORY_WRITE_CAPABILITY_ID,
            json!({
                "target": target,
                "content": "should not write"
            }),
            execution_context_with_mounts(
                [MEMORY_WRITE_CAPABILITY_ID],
                memory_mounts(MountPermissions::read_write_list_delete()),
            ),
        )
        .await
        .unwrap_err();
        assert_eq!(failure, RuntimeFailureKind::InvalidInput);
    }
}

#[tokio::test]
async fn memory_write_treats_null_target_as_omitted() {
    let runtime = runtime_with_filesystem(InMemoryBackend::new());
    let output = invoke_with_context(
        &runtime,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({
            "target": null,
            "content": "null target should use the default daily log"
        }),
        execution_context_with_mounts(
            [MEMORY_WRITE_CAPABILITY_ID],
            memory_mounts(MountPermissions::read_write_list_delete()),
        ),
    )
    .await
    .unwrap();
    assert_eq!(output["status"], json!("written"));
    assert_eq!(output["append"], json!(true));
    assert!(
        output["path"]
            .as_str()
            .is_some_and(|path| path.starts_with("daily/") && path.ends_with(".md")),
        "null target should default to today's daily log, got {output:?}"
    );
}

#[tokio::test]
async fn memory_read_returns_input_error_for_missing_document() {
    let runtime = runtime_with_filesystem(InMemoryBackend::new());
    let failure = invoke_with_context(
        &runtime,
        MEMORY_READ_CAPABILITY_ID,
        json!({"path": "projects/alpha/missing.md"}),
        execution_context_with_mounts(
            [MEMORY_READ_CAPABILITY_ID],
            memory_mounts(MountPermissions::read_write_list_delete()),
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(failure, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn memory_write_requires_memory_mount_authority() {
    let runtime = runtime_with_filesystem(InMemoryBackend::new());
    let (_filesystem, workspace_mounts) =
        in_memory_mounted_filesystem(MountPermissions::read_write_list_delete());
    let failure = invoke_with_context(
        &runtime,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({
            "target": "notes.md",
            "content": "should not write"
        }),
        execution_context_with_mounts([MEMORY_WRITE_CAPABILITY_ID], workspace_mounts),
    )
    .await
    .unwrap_err();
    assert_eq!(failure, RuntimeFailureKind::Authorization);
}

#[tokio::test]
async fn builtin_echo_invokes_through_host_runtime() {
    let output = invoke(ECHO_CAPABILITY_ID, json!({"message": "hello reborn"}))
        .await
        .unwrap();
    assert_eq!(output, Value::String("hello reborn".to_string()));
}

#[tokio::test]
async fn builtin_time_tolerates_null_string_sentinels_in_optional_fields() {
    // Weaker models (e.g. quantized local models) tend to fill every optional
    // parameter with the literal string "null" instead of omitting it. Those
    // sentinels in optional fields must be treated as absent, not abort the run.
    let output = invoke(
        TIME_CAPABILITY_ID,
        json!({
            "operation": "now",
            "timezone": "null",
            "from_timezone": "null",
            "format": "null"
        }),
    )
    .await
    .unwrap();
    assert!(
        output.get("iso").and_then(Value::as_str).is_some(),
        "time `now` should return an iso timestamp, got {output:?}"
    );
}

#[tokio::test]
async fn builtin_echo_preserves_null_string_in_required_field() {
    // The optional-sentinel normalization must never touch required fields, so a
    // deliberate "null" payload still round-trips unchanged.
    let output = invoke(ECHO_CAPABILITY_ID, json!({"message": "null"}))
        .await
        .unwrap();
    assert_eq!(output, Value::String("null".to_string()));
}

#[tokio::test]
async fn builtin_coding_tools_tolerate_null_sentinels_in_optional_fields() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("qa-builtins.md"), "Alpha\nBeta\nGamma\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(
        [
            LIST_DIR_CAPABILITY_ID,
            GLOB_CAPABILITY_ID,
            GREP_CAPABILITY_ID,
        ],
        mounts,
    );

    let listed = invoke_with_context(
        &runtime,
        LIST_DIR_CAPABILITY_ID,
        json!({
            "path": "/workspace",
            "recursive": "null",
            "max_depth": "null"
        }),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(listed["entries"], json!(["qa-builtins.md (17B)"]));

    let globbed = invoke_with_context(
        &runtime,
        GLOB_CAPABILITY_ID,
        json!({
            "path": "/workspace",
            "pattern": "qa-*.md",
            "max_results": "null"
        }),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(globbed["files"], json!(["qa-builtins.md"]));

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({
            "path": "/workspace",
            "pattern": "Beta",
            "context": "null",
            "before_context": "null",
            "after_context": "null",
            "head_limit": "null",
            "offset": "null"
        }),
        context,
    )
    .await
    .unwrap();
    assert_eq!(grepped["files"], json!(["qa-builtins.md"]));
}

#[tokio::test]
async fn builtin_spawn_subagent_authorization_invokes_through_host_runtime() {
    let output = invoke(SPAWN_SUBAGENT_CAPABILITY_ID, json!({}))
        .await
        .unwrap();
    assert_eq!(output, json!({"authorized": true}));
}

#[tokio::test]
async fn builtin_shell_invokes_copied_shell_core_through_host_runtime() {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_network([SHELL_CAPABILITY_ID], shell_test_policy()),
            capability_id(SHELL_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({"command": "echo hello reborn"}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed shell invocation, got {outcome:?}");
    };
    let output = &completed.output;
    assert_eq!(output["exit_code"], json!(0));
    assert_eq!(output["success"], json!(true));
    assert_eq!(output["sandboxed"], json!(false));
    assert!(
        output["output"]
            .as_str()
            .expect("shell output must be text")
            .contains("hello reborn")
    );
    assert_eq!(completed.usage.process_count, 1);
}

#[tokio::test]
async fn builtin_shell_delegates_command_execution_to_process_port() {
    let process_port = Arc::new(RecordingProcessPort::default());
    let runtime = runtime_with_process_port(Arc::clone(&process_port));

    let output = invoke_with_context(
        &runtime,
        SHELL_CAPABILITY_ID,
        json!({"command": "echo via port", "timeout": 9, "workdir": "port-workdir"}),
        execution_context_with_network([SHELL_CAPABILITY_ID], shell_test_policy()),
    )
    .await
    .unwrap();

    assert_eq!(output["exit_code"], json!(0));
    assert_eq!(output["success"], json!(true));
    assert_eq!(output["sandboxed"], json!(true));
    assert_eq!(output["output"], json!("process port: echo via port"));
    let requests = process_port.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].command, "echo via port");
    assert_eq!(requests[0].workdir.as_deref(), Some("port-workdir"));
    assert_eq!(requests[0].timeout_secs, Some(9));
    assert!(
        requests[0]
            .mounts
            .as_ref()
            .is_some_and(|mounts| mounts.mounts.is_empty())
    );
    assert!(requests[0].extra_env.is_empty());
    assert_eq!(requests[0].scope.user_id.as_str(), "user");
}

#[tokio::test]
async fn builtin_shell_returns_stderr_and_nonzero_exit_without_dispatch_failure() {
    let output = invoke_shell(json!({"command": "printf shell-error >&2; exit 7"}))
        .await
        .unwrap();

    assert_eq!(output["exit_code"], json!(7));
    assert_eq!(output["success"], json!(false));
    assert_eq!(output["sandboxed"], json!(false));
    assert_eq!(output["output"], json!("shell-error"));
}

#[tokio::test]
async fn builtin_shell_uses_configured_tenant_sandbox_process_port() {
    let local_process = Arc::new(RecordingProcessPort::default());
    let sandbox_transport = Arc::new(RecordingSandboxTransport::default());
    let sandbox_process = Arc::new(TenantSandboxProcessPort::new(sandbox_transport.clone()));
    let runtime = runtime_with_local_and_sandbox_process_ports(
        Arc::clone(&local_process),
        Arc::clone(&sandbox_process),
        tenant_sandbox_process_policy(),
    );

    let output = invoke_with_context(
        &runtime,
        SHELL_CAPABILITY_ID,
        json!({"command": "echo in sandbox"}),
        execution_context_with_network([SHELL_CAPABILITY_ID], shell_test_policy()),
    )
    .await
    .unwrap();

    assert_eq!(output["sandboxed"], json!(true));
    assert_eq!(output["output"], json!("process port: echo in sandbox"));
    assert!(local_process.requests.lock().unwrap().is_empty());
    assert_eq!(sandbox_transport.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn builtin_shell_rejects_hosted_process_plan_before_handler_runs() {
    let process_port = Arc::new(RecordingProcessPort::default());
    let runtime =
        runtime_with_process_port_and_policy(Arc::clone(&process_port), hosted_dev_policy());

    let error = invoke_with_context(
        &runtime,
        SHELL_CAPABILITY_ID,
        json!({"command": "echo must not run"}),
        execution_context_with_network([SHELL_CAPABILITY_ID], shell_test_policy()),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Authorization);
    assert!(
        process_port.requests.lock().unwrap().is_empty(),
        "hosted shell must fail at invocation-service resolution before the handler can run"
    );
}

#[tokio::test]
async fn builtin_shell_reuses_v1_shell_validation() {
    for input in [
        json!({"command": "cat ~/server.key"}),
        json!({"command": "printf '\\x65\\x63\\x68\\x6f hi'|dash"}),
        json!({"command": "wc < ~/server.key"}),
    ] {
        let err = invoke_shell(input).await.unwrap_err();

        assert_eq!(err, RuntimeFailureKind::Backend);
    }
}

#[tokio::test]
async fn builtin_shell_rejects_invalid_inputs_before_spawn() {
    for input in [
        json!({}),
        json!({"command": 123}),
        json!({"command": "echo hi", "workdir": 123}),
        json!({"command": "echo hi", "timeout": 0}),
        json!({"command": "echo hi", "timeout": "1"}),
    ] {
        let err = invoke_shell(input).await.unwrap_err();

        assert_eq!(err, RuntimeFailureKind::InvalidInput);
    }
}

#[tokio::test]
async fn builtin_shell_rejects_timeout_above_manifest_ceiling() {
    let err = invoke_shell(json!({"command": "echo hi", "timeout": 121}))
        .await
        .unwrap_err();

    assert_eq!(err, RuntimeFailureKind::Resource);
}

#[tokio::test]
async fn builtin_shell_maps_timeout_and_spawn_failures() {
    let timeout = invoke_shell(json!({"command": "sleep 2", "timeout": 1}))
        .await
        .unwrap_err();
    assert_eq!(timeout, RuntimeFailureKind::Resource);

    let spawn = invoke_shell(json!({
            "command": "echo missing",
            "workdir": "/definitely/missing/ironclaw-shell-test"
    }))
    .await
    .unwrap_err();
    assert_eq!(spawn, RuntimeFailureKind::Backend);
}

#[tokio::test]
async fn builtin_shell_truncates_large_output_without_output_overflow() {
    let output = invoke_shell(json!({
        "command": "i=0; while [ $i -lt 70000 ]; do printf x; i=$((i+1)); done",
        "timeout": 5
    }))
    .await
    .unwrap();

    let output = output["output"].as_str().expect("shell output is text");
    assert!(output.contains("[truncated"));
    assert!(output.contains("no file_read-accessible scoped path was available"));
    assert!(!output.contains(std::env::temp_dir().to_string_lossy().as_ref()));
    assert!(output.len() <= 66_000);
}

#[tokio::test]
async fn builtin_shell_saves_large_output_to_file_read_path() {
    let (filesystem, mounts) = in_memory_mounted_filesystem(MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts_and_network(
        [SHELL_CAPABILITY_ID, READ_FILE_CAPABILITY_ID],
        mounts,
        shell_test_policy(),
    );
    let shell_output = invoke_with_context(
        &runtime,
        SHELL_CAPABILITY_ID,
        json!({
            "command": "printf 'saved-start\\n'; yes m | head -c 70000; printf 'saved-end'",
            "timeout": 5
        }),
        context.clone(),
    )
    .await
    .unwrap();
    let output = shell_output["output"].as_str().expect("shell output text");
    let saved_path = output
        .split("Full output saved to: ")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .expect("saved output path");
    assert!(saved_path.starts_with("/workspace/command-outputs/"));
    assert!(!output.contains(std::env::temp_dir().to_string_lossy().as_ref()));
    assert!(output.contains("Use file_read to inspect it"));

    let read_output = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": saved_path, "limit": 1}),
        context,
    )
    .await
    .unwrap();
    assert!(
        read_output["content"]
            .as_str()
            .expect("read_file content")
            .contains("saved-start")
    );
    assert_eq!(read_output["path"], json!(saved_path));
}

#[tokio::test]
async fn builtin_shell_blocks_small_secret_output_through_dispatch() {
    let output = invoke_shell(json!({
        "command": "printf '%s' 'sk-proj-test1234567890abcdefghij'"
    }))
    .await
    .unwrap();

    assert_eq!(
        output["output"],
        json!("[Full command output blocked due to potential secret leakage]\n")
    );
}

#[tokio::test]
async fn builtin_shell_does_not_inherit_unlisted_parent_env() {
    static ENV_LOCK: LazyLock<tokio::sync::Mutex<()>> =
        LazyLock::new(|| tokio::sync::Mutex::new(()));
    let _guard = ENV_LOCK.lock().await;

    // SAFETY: test uses a crate-local mutex and a unique environment variable;
    // no production code depends on this key.
    unsafe {
        std::env::set_var("IRONCLAW_SHELL_SECRET_TEST", "must_not_leak");
    }
    let output = invoke_shell(json!({
        "command": "printf ${IRONCLAW_SHELL_SECRET_TEST:-missing}"
    }))
    .await;
    // SAFETY: clears only the test-owned key set above.
    unsafe {
        std::env::remove_var("IRONCLAW_SHELL_SECRET_TEST");
    }

    let output = output.unwrap();
    assert_eq!(output["output"], json!("missing"));
}

#[tokio::test]
async fn builtin_shell_rejects_scoped_mount_workdir_until_process_backend_handles_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut permissions = MountPermissions::read_write();
    permissions.execute = true;
    let (filesystem, mounts) = mounted_filesystem(temp.path(), permissions);
    let runtime = runtime_with_filesystem(filesystem);
    let error = invoke_with_context(
        &runtime,
        SHELL_CAPABILITY_ID,
        json!({"command": "pwd", "workdir": "/workspace"}),
        execution_context_with_mounts_and_network(
            [SHELL_CAPABILITY_ID],
            mounts,
            shell_test_policy(),
        ),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Backend);
}

#[tokio::test]
async fn builtin_time_parse_convert_and_diff_are_deterministic() {
    let parsed = invoke(
        TIME_CAPABILITY_ID,
        json!({"operation": "parse", "input": "2026-05-12T13:00:00Z"}),
    )
    .await
    .unwrap();
    assert_eq!(parsed["unix"], json!(1778590800));

    let converted = invoke(
        TIME_CAPABILITY_ID,
        json!({
            "operation": "convert",
            "input": "2026-05-12T13:00:00Z",
            "to_timezone": "America/New_York"
        }),
    )
    .await
    .unwrap();
    assert_eq!(converted["output"], json!("2026-05-12T09:00:00-04:00"));

    let diff = invoke(
        TIME_CAPABILITY_ID,
        json!({
            "operation": "diff",
            "input": "2026-05-12T13:00:00Z",
            "timestamp2": "2026-05-12T15:30:00Z"
        }),
    )
    .await
    .unwrap();
    assert_eq!(diff["minutes"], json!(150));
}

#[tokio::test]
async fn builtin_time_rejects_naive_without_timezone_and_ambiguous_local_time() {
    let missing_timezone = invoke(
        TIME_CAPABILITY_ID,
        json!({"operation": "parse", "input": "2026-05-12 13:00:00"}),
    )
    .await
    .unwrap_err();
    assert_eq!(missing_timezone, RuntimeFailureKind::InvalidInput);

    let ambiguous = invoke(
        TIME_CAPABILITY_ID,
        json!({
            "operation": "parse",
            "input": "2026-11-01 01:30:00",
            "timezone": "America/New_York"
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(ambiguous, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn builtin_json_parse_query_stringify_and_validate_work() {
    let parsed = invoke(
        JSON_CAPABILITY_ID,
        json!({"operation": "parse", "data": "{\"items\":[{\"name\":\"alpha\"}]}"}),
    )
    .await
    .unwrap();
    assert_eq!(parsed["items"][0]["name"], json!("alpha"));

    let queried = invoke(
        JSON_CAPABILITY_ID,
        json!({
            "operation": "query",
            "data": {"items":[{"name":"alpha"}]},
            "path": "items[0].name"
        }),
    )
    .await
    .unwrap();
    assert_eq!(queried, json!("alpha"));

    let valid = invoke(
        JSON_CAPABILITY_ID,
        json!({"operation": "validate", "data": "{\"ok\":true}"}),
    )
    .await
    .unwrap();
    assert_eq!(valid, json!({"valid": true}));

    let stringified = invoke(
        JSON_CAPABILITY_ID,
        json!({"operation": "stringify", "data": {"ok": true}}),
    )
    .await
    .unwrap();
    assert!(stringified.as_str().unwrap().contains("\"ok\": true"));
}

#[tokio::test]
async fn builtin_json_stringify_rejects_invalid_json_strings() {
    let error = invoke(
        JSON_CAPABILITY_ID,
        json!({"operation": "stringify", "data": "not json"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn builtin_json_rejects_v1_tool_output_stash_refs_without_leaking_input() {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context([JSON_CAPABILITY_ID]),
            capability_id(JSON_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({
                "operation": "parse",
                "source_tool_call_id": "call_RAW_SECRET_sk-provider-secret"
            }),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected sanitized failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
    let debug = format!("{failure:?}");
    assert!(!debug.contains("RAW_SECRET"));
    assert!(!debug.contains("sk-provider-secret"));
}

#[tokio::test]
async fn builtin_http_invokes_through_host_runtime_egress() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        br#"{"accepted":true}"#.to_vec(),
    ));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "method": "post",
            "url": "https://api.example.test/v1/items",
            "headers": {
                "content-type": "application/json",
                "x-request-id": "first-party-http"
            },
            "body": {"ok": true},
            "response_body_limit": 4096,
            "timeout_ms": 2500
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap_or_else(|error| {
        panic!(
            "expected HTTP egress success, got {error:?}; recorded requests: {:?}",
            egress.requests()
        )
    });

    assert_eq!(output["status"], json!(200));
    assert_eq!(output["body_text"], json!(r#"{"accepted":true}"#));
    assert_eq!(output["request_bytes"], json!(11));
    assert_eq!(output["response_bytes"], json!(17));

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.runtime, RuntimeKind::FirstParty);
    assert_eq!(request.capability_id, capability_id(HTTP_CAPABILITY_ID));
    assert_eq!(request.method, NetworkMethod::Post);
    assert_eq!(request.url, "https://api.example.test/v1/items");
    assert_eq!(request.body, br#"{"ok":true}"#);
    assert_eq!(request.response_body_limit, Some(4096));
    assert_eq!(request.save_body_to, None);
    assert_eq!(request.timeout_ms, Some(2500));
    assert!(request.credential_injections.is_empty());
}

#[tokio::test]
async fn builtin_http_requires_tool_call_http_egress_for_inline_output() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(b"ok".to_vec()));
    let runtime = runtime_with_strict_http_egress_only(Arc::clone(&egress));

    let failure = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items"
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap_err();

    assert_eq!(failure, RuntimeFailureKind::Network);
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn builtin_http_preserves_exact_body_cap_for_text_responses() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(vec![b'a'; 4096]));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "response_body_limit": 4096
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    let body_text = output["body_text"].as_str().expect("text response");
    assert_eq!(body_text.len(), 4096);
    assert_eq!(output.get("body_truncated"), None);
    assert!(output.get("body_base64").is_none());
    assert!(serialized_json_len(&output) <= 6_000);
}

#[tokio::test]
async fn builtin_http_truncates_one_byte_over_text_responses_with_a_hint() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(vec![b'a'; 4097]));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "response_body_limit": 4096
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    let body_text = output["body_text"].as_str().expect("text response");
    assert_eq!(body_text.len(), 4096);
    assert_eq!(output["body_truncated"], json!(true));
    assert_eq!(output["truncation"]["body"], json!(true));
    assert_eq!(output["truncation"]["headers"], json!(false));
    assert_eq!(output["truncation"]["bytes_returned"], json!(4096));
    assert_eq!(
        output["truncation"]["reason"],
        json!("model_visible_budget")
    );
    assert!(
        output["body_truncation_hint"]
            .as_str()
            .expect("hint")
            .contains("builtin.http.save")
    );
    assert!(output.get("body_base64").is_none());
    assert!(serialized_json_len(&output) <= 6_000);
}

#[tokio::test]
async fn builtin_http_default_large_response_result_stays_under_documented_cap() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(vec![
        b'a';
        1024 * 1024
    ]));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/large-page"
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    let body_text = output["body_text"].as_str().expect("text response");
    assert_eq!(body_text.len(), 48 * 1024);
    assert_eq!(output["body_truncated"], json!(true));
    assert!(
        output["body_truncation_hint"]
            .as_str()
            .expect("hint")
            .contains("builtin.http.save")
    );
    assert!(serialized_json_len(&output) <= 50 * 1024);

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].response_body_limit, Some(48 * 1024));
}

#[tokio::test]
async fn builtin_http_truncates_escaped_text_to_serialized_body_budget() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        "\n".repeat(4096).into_bytes(),
    ));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/escaped",
            "response_body_limit": 4096
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    let body_text = output["body_text"].as_str().expect("text response");
    let serialized_body = serde_json::to_string(body_text).unwrap();
    assert!(serialized_body.len().saturating_sub(2) <= 4096);
    assert_eq!(body_text.len(), 2048);
    assert_eq!(output["body_bytes_returned"], json!(2048));
    assert_eq!(output["body_truncated"], json!(true));
    assert_eq!(output["truncation"]["bytes_returned"], json!(2048));
}

#[tokio::test]
async fn builtin_http_save_passes_save_to_and_returns_saved_body_metadata() {
    let egress = Arc::new(
        RecordingRuntimeHttpEgress::with_body(br#"{"accepted":true}"#.to_vec())
            .with_saved_body("/workspace/response.json", 17),
    );
    let runtime = runtime_with_http_egress(Arc::clone(&egress));
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_write(),
    )])
    .unwrap();

    let output = invoke_with_context(
        &runtime,
        HTTP_SAVE_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "save_to": "/workspace/response.json"
        }),
        execution_context_with_mounts_and_network(
            [HTTP_SAVE_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap_or_else(|error| {
        panic!(
            "expected saved-body HTTP success, got {error:?}; recorded requests: {:?}",
            egress.requests()
        )
    });

    assert_eq!(output["status"], json!(200));
    assert_eq!(
        output["saved_body"],
        json!({
            "path": "/workspace/response.json",
            "bytes_written": 17
        })
    );
    assert!(output.get("body_text").is_none());
    assert!(output.get("body_base64").is_none());

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].response_body_limit, Some(10 * 1024 * 1024));
    let save_target = requests[0]
        .save_body_to
        .as_ref()
        .expect("save_to should be passed to host egress");
    assert_eq!(save_target.path.as_str(), "/workspace/response.json");
    let mount_grant = save_target
        .mount_grant
        .as_ref()
        .expect("save_to should carry narrowed mount authority");
    let mount_view = MountView::new(vec![mount_grant.clone()]).unwrap();
    let (virtual_path, grant) = mount_view
        .resolve_with_grant(&save_target.path)
        .expect("saved path should resolve through captured mount view");
    assert_eq!(
        virtual_path,
        VirtualPath::new("/projects/workspace/response.json").unwrap()
    );
    assert!(grant.permissions.write);
}

#[tokio::test]
async fn builtin_http_save_rejects_response_body_limit_above_save_ceiling_before_egress() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let runtime = runtime_with_http_egress(Arc::clone(&egress));
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_write(),
    )])
    .unwrap();

    let error = invoke_with_context(
        &runtime,
        HTTP_SAVE_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "save_to": "/workspace/response.json",
            "response_body_limit": 10 * 1024 * 1024 + 1
        }),
        execution_context_with_mounts_and_network(
            [HTTP_SAVE_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn builtin_http_save_returns_saved_body_for_large_responses_without_inline_body() {
    let egress = Arc::new(
        RecordingRuntimeHttpEgress::with_body(vec![b'a'; 12 * 1024])
            .with_saved_body("/workspace/large-response.json", 12 * 1024),
    );
    let runtime = runtime_with_http_egress(Arc::clone(&egress));
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_write(),
    )])
    .unwrap();

    let output = invoke_with_context(
        &runtime,
        HTTP_SAVE_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "save_to": "/workspace/large-response.json",
            "response_body_limit": 4096
        }),
        execution_context_with_mounts_and_network(
            [HTTP_SAVE_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap();

    assert_eq!(
        output["saved_body"],
        json!({
            "path": "/workspace/large-response.json",
            "bytes_written": 12 * 1024
        })
    );
    assert!(output.get("body_text").is_none());
    assert!(output.get("body_base64").is_none());
    assert!(serialized_json_len(&output) <= 2_000);

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].response_body_limit, Some(4096));
}

#[tokio::test]
async fn builtin_http_save_succeeds_with_strict_host_egress_only() {
    let egress = Arc::new(
        RecordingRuntimeHttpEgress::with_body(vec![b'a'; 12 * 1024])
            .with_saved_body("/workspace/strict-only-save.json", 12 * 1024),
    );
    let runtime = runtime_with_strict_http_egress_only(Arc::clone(&egress));
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_write(),
    )])
    .unwrap();

    let output = invoke_with_context(
        &runtime,
        HTTP_SAVE_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "save_to": "/workspace/strict-only-save.json",
            "response_body_limit": 4096
        }),
        execution_context_with_mounts_and_network(
            [HTTP_SAVE_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap();

    assert_eq!(
        output["saved_body"],
        json!({
            "path": "/workspace/strict-only-save.json",
            "bytes_written": 12 * 1024
        })
    );
    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].response_body_limit, Some(4096));
}

#[tokio::test]
async fn builtin_http_save_uses_strict_host_egress_when_tool_call_port_exists() {
    let strict_egress = Arc::new(
        RecordingRuntimeHttpEgress::with_body(vec![b'a'; 12 * 1024])
            .with_saved_body("/workspace/strict-save.json", 12 * 1024),
    );
    let runtime = HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_http_egress(Arc::clone(&strict_egress))
    .with_tool_call_http_egress(Arc::new(PanickingToolCallHttpEgress))
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing();
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_write(),
    )])
    .unwrap();

    let output = invoke_with_context(
        &runtime,
        HTTP_SAVE_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "save_to": "/workspace/strict-save.json",
            "response_body_limit": 4096
        }),
        execution_context_with_mounts_and_network(
            [HTTP_SAVE_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap();

    assert_eq!(
        output["saved_body"],
        json!({
            "path": "/workspace/strict-save.json",
            "bytes_written": 12 * 1024
        })
    );
    let requests = strict_egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].response_body_limit, Some(4096));
}

#[tokio::test]
async fn builtin_http_does_not_inline_huge_binary_payloads() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(vec![0xFF; 8 * 1024]));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "response_body_limit": 4096
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    let body_base64 = output["body_base64"].as_str().expect("binary response");
    assert_eq!(output["body_truncated"], json!(true));
    assert!(body_base64.len() <= 4096);
    assert_eq!(output["body_bytes_returned"], json!(3072));
    assert_eq!(output["truncation"]["body"], json!(true));
    assert_eq!(output["truncation"]["bytes_returned"], json!(3072));
    assert!(
        output["body_truncation_hint"]
            .as_str()
            .expect("hint")
            .contains("builtin.http.save")
    );
    assert!(output.get("body_text").is_none());
    assert!(serialized_json_len(&output) <= 6_000);
}

#[tokio::test]
async fn builtin_http_truncates_tiny_binary_responses_without_panicking() {
    for response_body_limit in 1..=3 {
        let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(vec![0xFF; 8]));
        let runtime = runtime_with_http_egress(Arc::clone(&egress));

        let output = invoke_with_context(
            &runtime,
            HTTP_CAPABILITY_ID,
            json!({
                "url": "https://api.example.test/v1/items",
                "response_body_limit": response_body_limit
            }),
            execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
        )
        .await
        .unwrap();

        assert_eq!(output["body_base64"], json!(""));
        assert_eq!(output["body_truncated"], json!(true));
        assert_eq!(output["body_bytes_returned"], json!(0));
        assert_eq!(output["truncation"]["body"], json!(true));
        assert_eq!(output["truncation"]["bytes_returned"], json!(0));
        assert!(serialized_json_len(&output) <= 2 * 1024 + response_body_limit as usize);
    }
}

#[tokio::test]
async fn builtin_http_final_budget_trim_preserves_base64_alignment() {
    let headers = (0..4)
        .map(|index| (format!("x-large-{index}"), "h".repeat(512)))
        .collect::<Vec<_>>();
    let egress =
        Arc::new(RecordingRuntimeHttpEgress::with_body(vec![0xFF; 4 * 1024]).with_headers(headers));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "response_body_limit": 4096
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    let body_base64 = output["body_base64"].as_str().expect("binary response");
    assert_eq!(output["body_truncated"], json!(true));
    assert!(!body_base64.is_empty());
    assert!(body_base64.len() < 4096);
    assert_eq!(body_base64.len() % 4, 0);
    assert_eq!(output["truncation"]["body"], json!(true));
    assert!(serialized_json_len(&output) <= 6_000);
}

#[tokio::test]
async fn builtin_http_final_budget_trims_headers_when_body_cannot_absorb_overage() {
    let headers = (0..32)
        .map(|index| (format!("x-large-{index}"), "h".repeat(1024)))
        .collect::<Vec<_>>();
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(Vec::new()).with_headers(headers));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "response_body_limit": 1
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    assert_eq!(output["headers_truncated"], json!(true));
    assert_eq!(output["truncation"]["headers"], json!(true));
    assert!(serialized_json_len(&output) <= 2 * 1024 + 1);
}

#[tokio::test]
async fn builtin_http_save_final_budget_trims_headers_without_inlining_body() {
    let headers = (0..32)
        .map(|index| (format!("x-large-{index}"), "h".repeat(1024)))
        .collect::<Vec<_>>();
    let egress = Arc::new(
        RecordingRuntimeHttpEgress::with_body(vec![b'a'; 12 * 1024])
            .with_saved_body("/workspace/header-heavy-save.json", 12 * 1024)
            .with_headers(headers),
    );
    let runtime = runtime_with_http_egress(Arc::clone(&egress));
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_write(),
    )])
    .unwrap();

    let output = invoke_with_context(
        &runtime,
        HTTP_SAVE_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "save_to": "/workspace/header-heavy-save.json",
            "response_body_limit": 1
        }),
        execution_context_with_mounts_and_network(
            [HTTP_SAVE_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap();

    assert!(output.get("body_text").is_none());
    assert!(output.get("body_base64").is_none());
    assert_eq!(output["headers_truncated"], json!(true));
    assert_eq!(output["truncation"]["headers"], json!(true));
    assert!(serialized_json_len(&output) <= 2 * 1024 + 1);
}

#[tokio::test]
async fn builtin_http_truncates_overlong_response_headers_to_model_visible_budget() {
    let mut headers = vec![(format!("x-{}", "n".repeat(200)), "\n".repeat(2 * 1024))];
    for index in 0..40 {
        headers.push((format!("x-extra-{index}"), "ok".to_string()));
    }
    let egress =
        Arc::new(RecordingRuntimeHttpEgress::with_body(b"ok".to_vec()).with_headers(headers));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "response_body_limit": 4096
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    let headers = output["headers"].as_array().expect("headers array");
    assert!(headers.len() <= 32);
    assert_eq!(output["headers_truncated"], json!(true));
    assert_eq!(output["truncation"]["headers"], json!(true));
    assert_eq!(output["truncation"]["body"], json!(false));
    assert_eq!(headers[0]["name"].as_str().unwrap().len(), 128);
    let header_value = headers[0]["value"].as_str().unwrap();
    assert_eq!(header_value.len(), 512);
    assert!(
        serde_json::to_string(header_value)
            .unwrap()
            .len()
            .saturating_sub(2)
            <= 1024
    );
    assert_eq!(headers[0]["truncated"], json!(true));
    assert!(serialized_json_len(&output["headers"]) <= 8 * 1024);
}

#[tokio::test]
async fn builtin_http_reports_body_and_header_truncation_together() {
    let mut headers = vec![(format!("x-{}", "n".repeat(200)), "\n".repeat(2 * 1024))];
    for index in 0..40 {
        headers.push((format!("x-extra-{index}"), "ok".to_string()));
    }
    let egress = Arc::new(
        RecordingRuntimeHttpEgress::with_body(vec![b'a'; 1024 * 1024]).with_headers(headers),
    );
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/large-page"
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    assert_eq!(output["headers_truncated"], json!(true));
    assert_eq!(output["body_truncated"], json!(true));
    assert_eq!(output["truncation"]["headers"], json!(true));
    assert_eq!(output["truncation"]["body"], json!(true));
    assert!(
        output["truncation"]["bytes_returned"]
            .as_u64()
            .expect("bytes returned")
            < 48 * 1024
    );
    assert!(output["headers"][0]["truncated"].as_bool().unwrap());
    assert!(serialized_json_len(&output) <= 50 * 1024);
}

#[tokio::test]
async fn builtin_http_keeps_sensitive_material_out_of_sanitized_output() {
    let egress = Arc::new(
        RecordingRuntimeHttpEgress::with_status_and_body(200, b"sanitized response body".to_vec())
            .with_headers(vec![
                (
                    "content-type".to_string(),
                    "text/plain; charset=utf-8".to_string(),
                ),
                ("x-request-id".to_string(), "sanitized-request".to_string()),
            ])
            .with_redaction_applied(true),
    );
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": {
                "authorization": "Bearer sk-provider-secret",
                "x-debug-token": "RAW_SECRET"
            },
            "body": {
                "token": "RAW_SECRET"
            },
            "response_body_limit": 4096
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    assert_eq!(output["redaction_applied"], json!(true));
    assert_eq!(output["headers"][0]["name"], json!("content-type"));
    assert_eq!(output["headers"][1]["value"], json!("sanitized-request"));

    let serialized = serde_json::to_string(&output).unwrap();
    assert!(!serialized.contains("sk-provider-secret"));
    assert!(!serialized.contains("RAW_SECRET"));
    assert!(serialized.contains("sanitized response body"));
}

#[tokio::test]
async fn builtin_http_does_not_report_redaction_as_truncation() {
    let egress = Arc::new(
        RecordingRuntimeHttpEgress::with_body(b"sanitized".to_vec())
            .with_redaction_applied(true)
            .with_response_bytes(1024),
    );
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "response_body_limit": 4096
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    assert_eq!(output["redaction_applied"], json!(true));
    assert_eq!(output["body_text"], json!("sanitized"));
    assert!(output.get("body_truncated").is_none());
    assert!(output.get("body_bytes_returned").is_none());
    assert!(output.get("truncation").is_none());
}

#[tokio::test]
async fn builtin_http_rejects_save_to_on_network_only_capability_before_egress() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let runtime = runtime_with_http_egress(Arc::clone(&egress));
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_write(),
    )])
    .unwrap();

    let error = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "save_to": "/workspace/response.json"
        }),
        execution_context_with_mounts_and_network([HTTP_CAPABILITY_ID], mounts, http_test_policy()),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn builtin_http_save_rejects_missing_save_to_before_egress() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let runtime = runtime_with_http_egress(Arc::clone(&egress));
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_write(),
    )])
    .unwrap();

    let error = invoke_with_context(
        &runtime,
        HTTP_SAVE_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items"
        }),
        execution_context_with_mounts_and_network(
            [HTTP_SAVE_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn builtin_http_save_rejects_save_to_without_mount_authority_before_egress() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let error = invoke_with_context(
        &runtime,
        HTTP_SAVE_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "save_to": "/workspace/response.json"
        }),
        execution_context_with_network([HTTP_SAVE_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn builtin_http_save_rejects_save_to_without_write_mount_before_egress() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let runtime = runtime_with_http_egress(Arc::clone(&egress));
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_only(),
    )])
    .unwrap();

    let error = invoke_with_context(
        &runtime,
        HTTP_SAVE_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "save_to": "/workspace/response.json"
        }),
        execution_context_with_mounts_and_network(
            [HTTP_SAVE_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn builtin_http_save_rejects_invalid_or_unresolved_save_to_before_egress() {
    for save_to in ["file:///tmp/response.json", "/other/response.json"] {
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let runtime = runtime_with_http_egress(Arc::clone(&egress));
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/workspace").unwrap(),
            MountPermissions::read_write(),
        )])
        .unwrap();

        let error = invoke_with_context(
            &runtime,
            HTTP_SAVE_CAPABILITY_ID,
            json!({
                "url": "https://api.example.test/v1/items",
                "save_to": save_to
            }),
            execution_context_with_mounts_and_network(
                [HTTP_SAVE_CAPABILITY_ID],
                mounts,
                http_test_policy(),
            ),
        )
        .await
        .unwrap_err();

        assert_eq!(error, RuntimeFailureKind::InvalidInput);
        assert!(egress.requests().is_empty());
    }
}

// arch-exempt: large-test-file, URL install tests share this first-party runtime harness; split plan #4062
#[tokio::test]
async fn builtin_skill_install_accepts_content_without_url_fetch() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let runtime = runtime_with_filesystem(filesystem);

    // `skill_install` advertises `EffectKind::Network` because the same
    // capability handles URL/GitHub installs, so `NetworkMode::Deny` rejects it
    // before content-only dispatch. This case keeps the non-fetch path covered.
    let installed = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"content": "---\nname: offline-helper\n---\nOffline prompt.\n"}),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts.clone(),
            http_test_policy(),
        ),
    )
    .await
    .unwrap();

    assert_eq!(installed["installed"], json!(true));
    assert_eq!(installed["name"], json!("offline-helper"));
    assert_eq!(installed["source"], json!("user"));

    let listed = invoke_with_context(
        &runtime,
        SKILL_LIST_CAPABILITY_ID,
        json!({}),
        execution_context_with_mounts([SKILL_LIST_CAPABILITY_ID], mounts),
    )
    .await
    .unwrap();
    assert_eq!(listed["count"], json!(1));
    assert_eq!(listed["skills"][0]["name"], json!("offline-helper"));
    assert_eq!(listed["skills"][0]["source"], json!("user"));
}

#[tokio::test]
async fn builtin_skill_install_accepts_and_replays_named_plain_markdown_content() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let runtime = runtime_with_filesystem(filesystem);
    let input = json!({
        "name": "daily digest email docs",
        "content": "# Daily Digest\n\nSummarize updates for an email.\n"
    });

    let installed = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        input.clone(),
        // skill_install currently declares Network for URL installs too, so the
        // first-party harness needs a non-empty policy even for content input.
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts.clone(),
            http_test_policy(),
        ),
    )
    .await
    .unwrap();

    assert_eq!(installed["installed"], json!(true));
    assert_eq!(installed["name"], json!("daily-digest-email-docs"));
    assert_eq!(installed["source"], json!("user"));

    let replayed = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        input,
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts.clone(),
            http_test_policy(),
        ),
    )
    .await
    .unwrap();

    assert_eq!(replayed["installed"], json!(true));
    assert_eq!(replayed["name"], json!("daily-digest-email-docs"));

    let listed = invoke_with_context(
        &runtime,
        SKILL_LIST_CAPABILITY_ID,
        json!({}),
        execution_context_with_mounts([SKILL_LIST_CAPABILITY_ID], mounts),
    )
    .await
    .unwrap();
    assert_eq!(listed["count"], json!(1));
    assert_eq!(
        listed["skills"][0]["name"],
        json!("daily-digest-email-docs")
    );
}

#[tokio::test]
async fn builtin_skill_install_rejects_hidden_url_install_fields() {
    let cases = [
        json!({
            "content": "---\nname: hidden-files\n---\nPrompt.\n",
            "files": [{"path": "references/injected.md", "bytes_base64": "IyBJbmplY3RlZAo="}]
        }),
        json!({
            "content": "---\nname: hidden-source\n---\nPrompt.\n",
            "source": "installed_url"
        }),
        json!({
            "content": "---\nname: hidden-source-url\n---\nPrompt.\n",
            "source_url": "https://api.example.test/skills/hidden-source-url/SKILL.md"
        }),
    ];

    for input in cases {
        let temp = tempfile::tempdir().unwrap();
        let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
        let runtime = runtime_with_filesystem(filesystem);

        let error = invoke_with_context(
            &runtime,
            SKILL_INSTALL_CAPABILITY_ID,
            input,
            execution_context_with_mounts_and_network(
                [SKILL_INSTALL_CAPABILITY_ID],
                mounts,
                http_test_policy(),
            ),
        )
        .await
        .unwrap_err();

        assert_eq!(error, RuntimeFailureKind::InvalidInput);
        assert!(temp.path().read_dir().unwrap().next().is_none());
    }
}

// URL-install coverage stays in this integration file because these cases assert
// the first-party runtime dispatch path, not only the URL parser helpers.
#[tokio::test]
async fn builtin_skill_install_url_path_fetches_through_host_runtime_egress() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        b"---\nname: fetched-helper\ndescription: fetched skill\n---\nFetched prompt.\n".to_vec(),
    ));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));
    let context = execution_context_with_mounts_and_network(
        [SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID],
        mounts,
        http_test_policy(),
    );

    let installed = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/fetched-helper/SKILL.md"}),
        context.clone(),
    )
    .await
    .unwrap();

    assert_eq!(installed["installed"], json!(true));
    assert_eq!(installed["name"], json!("fetched-helper"));
    assert_eq!(installed["path"], json!("/skills/fetched-helper/SKILL.md"));
    assert_eq!(installed["source"], json!("installed"));

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].runtime, RuntimeKind::FirstParty);
    assert_eq!(
        requests[0].capability_id,
        capability_id(SKILL_INSTALL_CAPABILITY_ID)
    );
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert_eq!(
        requests[0].url,
        "https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/fetched-helper/SKILL.md"
    );
    assert_eq!(requests[0].response_body_limit, Some(10 * 1024 * 1024));
    assert_eq!(requests[0].timeout_ms, Some(10_000));

    let listed = invoke_with_context(&runtime, SKILL_LIST_CAPABILITY_ID, json!({}), context)
        .await
        .unwrap();
    assert_eq!(listed["count"], json!(1));
    assert_eq!(listed["skills"][0]["name"], json!("fetched-helper"));
    assert_eq!(listed["skills"][0]["source"], json!("installed"));
}

#[tokio::test]
async fn builtin_skill_install_url_path_ignores_caller_supplied_hidden_bundle_files() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        b"---\nname: fetched-helper\ndescription: fetched skill\n---\nFetched prompt.\n".to_vec(),
    ));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, egress);

    let installed = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({
            "url": "https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/fetched.md",
            "files": [{
                "path": "references/injected.md",
                "bytes": [35, 32, 73, 110, 106, 101, 99, 116, 101, 100, 10],
            }],
        }),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap();

    assert_eq!(installed["files_installed"], json!(0));
    assert!(
        !temp
            .path()
            .join("fetched-helper/references/injected.md")
            .exists()
    );
}

#[tokio::test]
async fn builtin_skill_install_url_path_serializes_concurrent_fetches_from_same_url() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        b"---\nname: concurrent-helper\ndescription: fetched skill\n---\nFetched prompt.\n"
            .to_vec(),
    ));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));
    let context = execution_context_with_mounts_and_network(
        [SKILL_INSTALL_CAPABILITY_ID],
        mounts,
        http_test_policy(),
    );
    let url =
        "https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/concurrent-helper/SKILL.md";

    let (first, second) = tokio::join!(
        invoke_with_context(
            &runtime,
            SKILL_INSTALL_CAPABILITY_ID,
            json!({"url": url}),
            context.clone(),
        ),
        invoke_with_context(
            &runtime,
            SKILL_INSTALL_CAPABILITY_ID,
            json!({"url": url}),
            context,
        )
    );
    let mut outcomes = [first.map(|_| ()), second.map(|_| ())];
    outcomes.sort_by_key(|result| result.is_err());

    assert!(outcomes[0].is_ok());
    assert_eq!(outcomes[1], Err(RuntimeFailureKind::OperationFailed));
    assert_eq!(egress.requests().len(), 2);
    assert!(temp.path().join("concurrent-helper/SKILL.md").exists());
}

#[tokio::test]
async fn builtin_skill_install_url_path_installs_zip_bundle_supporting_files() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(skill_bundle_zip(&[
        (
            "bundle-main/zip-helper/SKILL.md",
            b"---\nname: zip-helper\ndescription: bundled skill\n---\nZip prompt.\n",
        ),
        (
            "bundle-main/zip-helper/references/guide.md",
            b"# Guide\nUse carefully.\n",
        ),
        ("bundle-main/zip-helper/scripts/run.py", b"print('ok')\n"),
    ])));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

    let installed = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "https://codeload.github.com/Pika-Labs/Pika-Skills/legacy.zip/main"}),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap();

    assert_eq!(installed["installed"], json!(true));
    assert_eq!(installed["name"], json!("zip-helper"));
    assert_eq!(installed["files_installed"], json!(2));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("zip-helper/references/guide.md")).unwrap(),
        "# Guide\nUse carefully.\n"
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("zip-helper/scripts/run.py")).unwrap(),
        "print('ok')\n"
    );
}

#[tokio::test]
async fn builtin_skill_install_url_path_installs_github_repo_bundle_supporting_files() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let archive = skill_bundle_zip(&[
        (
            "Pika-Skills-main/pikastream-video-meeting/SKILL.md",
            b"---\nname: pikastream-video-meeting\ndescription: repo skill\n---\nRepo prompt.\n",
        ),
        (
            "Pika-Skills-main/pikastream-video-meeting/scripts/run.py",
            b"print('repo')\n",
        ),
    ]);
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_url_bodies(BTreeMap::from([
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills".to_string(),
            (
                200,
                br#"{"default_branch":"main"}"#.to_vec(),
            ),
        ),
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/commits?sha=main&per_page=1"
                .to_string(),
            (
                200,
                br#"[{"sha":"abcdef0123456789abcdef0123456789abcdef01"}]"#.to_vec(),
            ),
        ),
        (
            "https://codeload.github.com/Pika-Labs/Pika-Skills/legacy.zip/abcdef0123456789abcdef0123456789abcdef01".to_string(),
            (200, archive),
        ),
    ])));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

    let installed = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "https://github.com/Pika-Labs/Pika-Skills"}),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap();

    assert_eq!(installed["name"], json!("pikastream-video-meeting"));
    assert_eq!(installed["files_installed"], json!(1));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("pikastream-video-meeting/scripts/run.py"))
            .unwrap(),
        "print('repo')\n"
    );
    assert_eq!(egress.requests().len(), 3);
}

#[tokio::test]
async fn builtin_skill_install_url_path_installs_github_blob_skill_md() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_url_bodies(BTreeMap::from([
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/git/matching-refs/heads/main"
                .to_string(),
            (
                200,
                br#"[{"ref":"refs/heads/main"}]"#.to_vec(),
            ),
        ),
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/contents/SKILL.md?ref=main"
                .to_string(),
            (
                200,
                br#"{"type":"file","download_url":"https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/SKILL.md"}"#.to_vec(),
            ),
        ),
        (
            "https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/SKILL.md".to_string(),
            (
                200,
                b"---\nname: blob-helper\ndescription: blob skill\n---\nBlob prompt.\n".to_vec(),
            ),
        ),
    ])));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

    let installed = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "https://github.com/Pika-Labs/Pika-Skills/blob/main/SKILL.md"}),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap();

    assert_eq!(installed["name"], json!("blob-helper"));
    assert_eq!(installed["source"], json!("installed"));
    assert_eq!(egress.requests().len(), 3);
}

#[tokio::test]
async fn builtin_skill_install_url_path_installs_github_tree_subdir_bundle_supporting_files() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let entries = br#"[
        {"type":"file","path":"skills/foo/SKILL.md"},
        {"type":"file","path":"skills/foo/references/tree.md"}
    ]"#
    .to_vec();
    let raw_skill =
        b"---\nname: tree-helper\ndescription: tree skill\n---\nTree prompt.\n".to_vec();
    let raw_reference = b"# Tree\n".to_vec();
    let commit_sha = "abcdef0123456789abcdef0123456789abcdef01";
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_url_bodies(BTreeMap::from([
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/git/matching-refs/heads/release"
                .to_string(),
            (
                200,
                br#"[{"ref":"refs/heads/release/v1"}]"#.to_vec(),
            ),
        ),
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/commits?sha=release%2Fv1&per_page=1"
                .to_string(),
            (
                200,
                br#"[{"sha":"abcdef0123456789abcdef0123456789abcdef01"}]"#.to_vec(),
            ),
        ),
        (
            format!("https://api.github.com/repos/Pika-Labs/Pika-Skills/contents/skills/foo?ref={commit_sha}"),
            (200, entries),
        ),
        (
            format!("https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/{commit_sha}/skills/foo/SKILL.md"),
            (200, raw_skill),
        ),
        (
            format!("https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/{commit_sha}/skills/foo/references/tree.md"),
            (200, raw_reference),
        ),
    ])));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

    let installed = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "https://github.com/Pika-Labs/Pika-Skills/tree/release/v1/skills/foo"}),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap();

    assert_eq!(installed["name"], json!("tree-helper"));
    assert_eq!(installed["files_installed"], json!(1));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("tree-helper/references/tree.md")).unwrap(),
        "# Tree\n"
    );
    let requests = egress.requests();
    assert_eq!(requests.len(), 5);
    assert!(
        requests
            .iter()
            .all(|request| !request.url.contains("codeload.github.com"))
    );
    assert!(
        requests
            .iter()
            .all(|request| !request.url.contains("release/v1/skills/foo"))
    );
}

#[tokio::test]
async fn builtin_skill_install_url_path_installs_github_tree_tag_subdir_bundle_supporting_files() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let commit_sha = "1234567890abcdef1234567890abcdef12345678";
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_url_bodies(BTreeMap::from([
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/git/matching-refs/heads/release"
                .to_string(),
            (200, b"[]".to_vec()),
        ),
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/git/matching-refs/tags/release"
                .to_string(),
            (
                200,
                br#"[{"ref":"refs/tags/release/v1"}]"#.to_vec(),
            ),
        ),
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/commits?sha=release%2Fv1&per_page=1"
                .to_string(),
            (
                200,
                br#"[{"sha":"1234567890abcdef1234567890abcdef12345678"}]"#.to_vec(),
            ),
        ),
        (
            format!("https://api.github.com/repos/Pika-Labs/Pika-Skills/contents/skills/foo?ref={commit_sha}"),
            (
                200,
                br#"[{"type":"file","path":"skills/foo/SKILL.md"}]"#.to_vec(),
            ),
        ),
        (
            format!("https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/{commit_sha}/skills/foo/SKILL.md"),
            (
                200,
                b"---\nname: tagged-tree-helper\ndescription: tree skill\n---\nTree prompt.\n".to_vec(),
            ),
        ),
    ])));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

    let installed = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "https://github.com/Pika-Labs/Pika-Skills/tree/release/v1/skills/foo"}),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap();

    assert_eq!(installed["name"], json!("tagged-tree-helper"));
    assert_eq!(egress.requests().len(), 5);
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_github_tree_directory_fanout() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let commit_sha = "abcdef0123456789abcdef0123456789abcdef01";
    let mut responses = BTreeMap::from([
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/git/matching-refs/heads/main"
                .to_string(),
            (200, br#"[{"ref":"refs/heads/main"}]"#.to_vec()),
        ),
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/commits?sha=main&per_page=1"
                .to_string(),
            (
                200,
                br#"[{"sha":"abcdef0123456789abcdef0123456789abcdef01"}]"#.to_vec(),
            ),
        ),
    ]);
    let mut directory = "skills/foo".to_string();
    for index in 0..64 {
        let next = format!("{directory}/dir-{index}");
        responses.insert(
            format!(
                "https://api.github.com/repos/Pika-Labs/Pika-Skills/contents/{directory}?ref={commit_sha}"
            ),
            (
                200,
                format!(r#"[{{"type":"dir","path":"{next}"}}]"#).into_bytes(),
            ),
        );
        directory = next;
    }
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_url_bodies(responses));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

    let error = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "https://github.com/Pika-Labs/Pika-Skills/tree/main/skills/foo"}),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::OutputTooLarge);
    assert_eq!(egress.requests().len(), 24);
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_invalid_zip_bundles() {
    let oversized_entry = vec![b'x'; 2 * 1024 * 1024 + 1];
    let too_many_entries = (0..(ironclaw_skills::MAX_INSTALL_BUNDLE_FILES * 4 + 1))
        .map(|index| (format!("bundle/empty-{index}.txt"), Vec::new()))
        .collect::<Vec<_>>();
    let too_many_directories = (0..(ironclaw_skills::MAX_INSTALL_BUNDLE_FILES * 4 + 1))
        .map(|index| format!("bundle/dir-{index}/"))
        .collect::<Vec<_>>();
    let oversized_total = (0..11)
        .map(|index| {
            (
                format!("bundle/big-{index}.bin"),
                vec![b'x'; 2 * 1024 * 1024],
            )
        })
        .collect::<Vec<_>>();
    let cases = [
        (
            skill_bundle_zip(&[("bundle/references/guide.md", b"# Guide\n")]),
            RuntimeFailureKind::OperationFailed,
        ),
        (
            skill_bundle_zip(&[
                ("bundle/one/SKILL.md", b"---\nname: one\n---\nOne\n"),
                ("bundle/two/SKILL.md", b"---\nname: two\n---\nTwo\n"),
            ]),
            RuntimeFailureKind::InvalidInput,
        ),
        (
            skill_bundle_zip(&[("../escape/SKILL.md", b"---\nname: escape\n---\nEscape\n")]),
            RuntimeFailureKind::InvalidInput,
        ),
        (
            skill_bundle_zip_owned([("bundle/SKILL.md".to_string(), vec![0xff, 0xfe, 0xfd])]),
            RuntimeFailureKind::OperationFailed,
        ),
        (
            skill_bundle_zip_owned([("bundle/oversized.bin".to_string(), oversized_entry)]),
            RuntimeFailureKind::OutputTooLarge,
        ),
        (
            skill_bundle_zip_owned(too_many_entries),
            RuntimeFailureKind::OutputTooLarge,
        ),
        (
            skill_bundle_zip_with_dirs(
                &too_many_directories,
                std::iter::empty::<(String, Vec<u8>)>(),
            ),
            RuntimeFailureKind::OutputTooLarge,
        ),
        (
            skill_bundle_zip_owned(
                std::iter::once((
                    "bundle/SKILL.md".to_string(),
                    b"---\nname: oversized-total\n---\nPrompt\n".to_vec(),
                ))
                .chain(oversized_total),
            ),
            RuntimeFailureKind::OutputTooLarge,
        ),
    ];

    for (body, expected) in cases {
        let temp = tempfile::tempdir().unwrap();
        let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
        let runtime = runtime_with_filesystem_and_http_egress(
            filesystem,
            Arc::new(RecordingRuntimeHttpEgress::with_body(body)),
        );

        let actual = invoke_with_context(
            &runtime,
            SKILL_INSTALL_CAPABILITY_ID,
            json!({"url": "https://codeload.github.com/Pika-Labs/Pika-Skills/legacy.zip/invalid"}),
            execution_context_with_mounts_and_network(
                [SKILL_INSTALL_CAPABILITY_ID],
                mounts,
                http_test_policy(),
            ),
        )
        .await
        .unwrap_err();

        assert_eq!(actual, expected);
        assert!(temp.path().read_dir().unwrap().next().is_none());
    }
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_truncated_zip_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        b"PK\x03\x04truncated".to_vec(),
    ));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

    let error = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "https://codeload.github.com/Pika-Labs/Pika-Skills/legacy.zip/truncated"}),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::OperationFailed);
    assert_eq!(egress.requests().len(), 1);
    assert!(temp.path().read_dir().unwrap().next().is_none());
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_invalid_github_api_responses() {
    let cases = [
        BTreeMap::from([(
            "https://api.github.com/repos/Pika-Labs/Pika-Skills".to_string(),
            (200, b"not json".to_vec()),
        )]),
        BTreeMap::from([(
            "https://api.github.com/repos/Pika-Labs/Pika-Skills".to_string(),
            (200, br#"{"name":"Pika-Skills"}"#.to_vec()),
        )]),
        BTreeMap::from([
            (
                "https://api.github.com/repos/Pika-Labs/Pika-Skills".to_string(),
                (200, br#"{"default_branch":"main"}"#.to_vec()),
            ),
            (
                "https://api.github.com/repos/Pika-Labs/Pika-Skills/commits?sha=main&per_page=1"
                    .to_string(),
                (200, br#"[{"sha":"not-a-sha"}]"#.to_vec()),
            ),
        ]),
    ];

    for responses in cases {
        let temp = tempfile::tempdir().unwrap();
        let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
        let runtime = runtime_with_filesystem_and_http_egress(
            filesystem,
            Arc::new(RecordingRuntimeHttpEgress::with_url_bodies(responses)),
        );

        let actual = invoke_with_context(
            &runtime,
            SKILL_INSTALL_CAPABILITY_ID,
            json!({"url": "https://github.com/Pika-Labs/Pika-Skills"}),
            execution_context_with_mounts_and_network(
                [SKILL_INSTALL_CAPABILITY_ID],
                mounts,
                http_test_policy(),
            ),
        )
        .await
        .unwrap_err();

        assert_eq!(actual, RuntimeFailureKind::OperationFailed);
        assert!(temp.path().read_dir().unwrap().next().is_none());
    }
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_github_tree_file_response() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let commit_sha = "abcdef0123456789abcdef0123456789abcdef01";
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_url_bodies(BTreeMap::from([
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/git/matching-refs/heads/main"
                .to_string(),
            (200, br#"[{"ref":"refs/heads/main"}]"#.to_vec()),
        ),
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/commits?sha=main&per_page=1"
                .to_string(),
            (
                200,
                br#"[{"sha":"abcdef0123456789abcdef0123456789abcdef01"}]"#.to_vec(),
            ),
        ),
        (
            format!("https://api.github.com/repos/Pika-Labs/Pika-Skills/contents/SKILL.md?ref={commit_sha}"),
            (
                200,
                br#"{"type":"file","path":"SKILL.md","download_url":"https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/SKILL.md"}"#.to_vec(),
            ),
        ),
    ])));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

    let error = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "https://github.com/Pika-Labs/Pika-Skills/tree/main/SKILL.md"}),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(temp.path().read_dir().unwrap().next().is_none());
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_github_blob_download_url_host_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_url_bodies(BTreeMap::from([
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/git/matching-refs/heads/main"
                .to_string(),
            (200, br#"[{"ref":"refs/heads/main"}]"#.to_vec()),
        ),
        (
            "https://api.github.com/repos/Pika-Labs/Pika-Skills/contents/SKILL.md?ref=main"
                .to_string(),
            (
                200,
                br#"{"type":"file","download_url":"https://api.github.com/repos/Pika-Labs/Pika-Skills/contents/secret"}"#.to_vec(),
            ),
        ),
    ])));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

    let error = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "https://github.com/Pika-Labs/Pika-Skills/blob/main/SKILL.md"}),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert_eq!(egress.requests().len(), 2);
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_malformed_github_paths_before_fetch() {
    for url in [
        "https://github.com/%24%7BINJECTION%7D/Pika-Skills",
        "https://github.com/Pika-Labs/Pika-Skills/releases",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
        let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            b"---\nname: fetched-helper\n---\nFetched prompt.\n".to_vec(),
        ));
        let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

        let error = invoke_with_context(
            &runtime,
            SKILL_INSTALL_CAPABILITY_ID,
            json!({"url": url}),
            execution_context_with_mounts_and_network(
                [SKILL_INSTALL_CAPABILITY_ID],
                mounts,
                http_test_policy(),
            ),
        )
        .await
        .unwrap_err();

        assert_eq!(error, RuntimeFailureKind::InvalidInput);
        assert!(egress.requests().is_empty());
    }
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_ambiguous_content_and_url_before_fetch() {
    for content in [
        json!("---\nname: pasted-helper\n---\nPasted prompt.\n"),
        json!(123),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
        let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            b"---\nname: fetched-helper\n---\nFetched prompt.\n".to_vec(),
        ));
        let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

        let error = invoke_with_context(
            &runtime,
            SKILL_INSTALL_CAPABILITY_ID,
            json!({
                "url": "https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/fetched-helper/SKILL.md",
                "content": content
            }),
            execution_context_with_mounts_and_network(
                [SKILL_INSTALL_CAPABILITY_ID],
                mounts,
                http_test_policy(),
            ),
        )
        .await
        .unwrap_err();

        assert_eq!(error, RuntimeFailureKind::InvalidInput);
        assert!(egress.requests().is_empty());
    }
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_non_https_url_before_fetch() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        b"---\nname: fetched-helper\n---\nFetched prompt.\n".to_vec(),
    ));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

    let error = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "http://api.example.test/skills/fetched-helper/SKILL.md"}),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_empty_input_before_fetch() {
    for input in [json!({}), json!({"name": "missing-url"})] {
        let temp = tempfile::tempdir().unwrap();
        let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
        let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            b"---\nname: fetched-helper\n---\nFetched prompt.\n".to_vec(),
        ));
        let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

        let error = invoke_with_context(
            &runtime,
            SKILL_INSTALL_CAPABILITY_ID,
            input,
            execution_context_with_mounts_and_network(
                [SKILL_INSTALL_CAPABILITY_ID],
                mounts,
                http_test_policy(),
            ),
        )
        .await
        .unwrap_err();

        assert_eq!(error, RuntimeFailureKind::InvalidInput);
        assert!(egress.requests().is_empty());
    }
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_whitespace_url_before_fetch() {
    for url in ["", "   "] {
        let temp = tempfile::tempdir().unwrap();
        let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
        let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            b"---\nname: fetched-helper\n---\nFetched prompt.\n".to_vec(),
        ));
        let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

        let error = invoke_with_context(
            &runtime,
            SKILL_INSTALL_CAPABILITY_ID,
            json!({"url": url}),
            execution_context_with_mounts_and_network(
                [SKILL_INSTALL_CAPABILITY_ID],
                mounts,
                http_test_policy(),
            ),
        )
        .await
        .unwrap_err();

        assert_eq!(error, RuntimeFailureKind::InvalidInput);
        assert!(egress.requests().is_empty());
    }
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_credentials_in_url_before_fetch() {
    for url in [
        "https://user@raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/SKILL.md",
        "https://user:pass@raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/SKILL.md",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
        let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            b"---\nname: fetched-helper\n---\nFetched prompt.\n".to_vec(),
        ));
        let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

        let error = invoke_with_context(
            &runtime,
            SKILL_INSTALL_CAPABILITY_ID,
            json!({"url": url}),
            execution_context_with_mounts_and_network(
                [SKILL_INSTALL_CAPABILITY_ID],
                mounts,
                http_test_policy(),
            ),
        )
        .await
        .unwrap_err();

        assert_eq!(error, RuntimeFailureKind::InvalidInput);
        assert!(egress.requests().is_empty());
    }
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_disallowed_hosts_before_fetch() {
    for url in [
        "https://169.254.169.254/latest/meta-data",
        "https://internal.service.local/skills/SKILL.md",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
        let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            b"---\nname: fetched-helper\n---\nFetched prompt.\n".to_vec(),
        ));
        let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

        let error = invoke_with_context(
            &runtime,
            SKILL_INSTALL_CAPABILITY_ID,
            json!({"url": url}),
            execution_context_with_mounts_and_network(
                [SKILL_INSTALL_CAPABILITY_ID],
                mounts,
                http_test_policy(),
            ),
        )
        .await
        .unwrap_err();

        assert_eq!(error, RuntimeFailureKind::InvalidInput);
        assert!(egress.requests().is_empty());
    }
}

#[tokio::test]
async fn builtin_skill_install_url_path_fails_closed_when_runtime_egress_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let runtime = runtime_with_filesystem_without_http_egress(filesystem);

    let error = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/fetched-helper/SKILL.md"}),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Network);
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_non_success_url_response() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_status_and_body(
        404,
        b"not found".to_vec(),
    ));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));
    let context = execution_context_with_mounts_and_network(
        [SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID],
        mounts,
        http_test_policy(),
    );

    let error = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/missing/SKILL.md"}),
        context.clone(),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::OperationFailed);
    assert_eq!(egress.requests().len(), 1);

    let listed = invoke_with_context(&runtime, SKILL_LIST_CAPABILITY_ID, json!({}), context)
        .await
        .unwrap();
    assert_eq!(listed["count"], json!(0));
}

#[tokio::test]
async fn builtin_skill_install_url_path_rejects_invalid_utf8_url_response() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(vec![0xff, 0xfe]));
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, Arc::clone(&egress));

    let error = invoke_with_context(
        &runtime,
        SKILL_INSTALL_CAPABILITY_ID,
        json!({"url": "https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/binary/SKILL.md"}),
        execution_context_with_mounts_and_network(
            [SKILL_INSTALL_CAPABILITY_ID],
            mounts,
            http_test_policy(),
        ),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::OperationFailed);
    assert_eq!(egress.requests().len(), 1);
}

#[tokio::test]
async fn builtin_skill_install_url_path_maps_runtime_egress_errors_by_reason() {
    let cases = [
        (
            RuntimeHttpEgressError::Request {
                reason: "sensitive_header_denied:authorization".to_string(),
                request_bytes: 0,
                response_bytes: 0,
            },
            RuntimeFailureKind::InvalidInput,
        ),
        (
            RuntimeHttpEgressError::Network {
                reason: "policy_denied".to_string(),
                request_bytes: 0,
                response_bytes: 0,
            },
            RuntimeFailureKind::PolicyDenied,
        ),
        (
            RuntimeHttpEgressError::Network {
                reason: "network_unreachable".to_string(),
                request_bytes: 0,
                response_bytes: 0,
            },
            RuntimeFailureKind::Network,
        ),
        (
            RuntimeHttpEgressError::Response {
                reason: "response_decode_failed".to_string(),
                request_bytes: 4,
                response_bytes: 1024,
            },
            RuntimeFailureKind::OperationFailed,
        ),
        (
            RuntimeHttpEgressError::Response {
                reason: "response_body_limit_exceeded".to_string(),
                request_bytes: 4,
                response_bytes: 1024,
            },
            RuntimeFailureKind::OutputTooLarge,
        ),
    ];

    for (error, expected) in cases {
        let temp = tempfile::tempdir().unwrap();
        let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
        let runtime = runtime_with_filesystem_and_http_egress(
            filesystem,
            Arc::new(RecordingRuntimeHttpEgress::with_error(error)),
        );
        let actual = invoke_with_context(
            &runtime,
            SKILL_INSTALL_CAPABILITY_ID,
            json!({"url": "https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/fetched-helper/SKILL.md"}),
            execution_context_with_mounts_and_network(
                [SKILL_INSTALL_CAPABILITY_ID],
                mounts,
                http_test_policy(),
            ),
        )
        .await
        .unwrap_err();
        assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn builtin_skill_install_url_path_accounts_wall_clock_time() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_skill_filesystem(temp.path());
    let egress = Arc::new(SleepingRuntimeHttpEgress {
        delay: Duration::from_millis(20),
        body: b"---\nname: slow-helper\n---\nSlow prompt.\n".to_vec(),
    });
    let runtime = runtime_with_filesystem_and_http_egress(filesystem, egress);
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_mounts_and_network(
                [SKILL_INSTALL_CAPABILITY_ID],
                mounts,
                http_test_policy(),
            ),
            capability_id(SKILL_INSTALL_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({"url": "https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/slow-helper/SKILL.md"}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed skill URL install, got {outcome:?}");
    };
    assert!(completed.usage.wall_clock_ms >= 10);
}

#[tokio::test]
async fn builtin_http_clamps_oversized_timeout_to_runtime_ceiling() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(b"ok".to_vec()));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "timeout_ms": 120_000
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout_ms, Some(30_000));
}

#[tokio::test]
async fn builtin_http_runtime_policy_denial_stops_before_egress() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        br#"{"ok":true}"#.to_vec(),
    ));
    let runtime = runtime_with_http_egress_and_policy(Arc::clone(&egress), network_denied_policy());

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
            capability_id(HTTP_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({"url": "https://api.example.test/v1/items"}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected runtime-policy failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
    assert_eq!(failure.capability_id, capability_id(HTTP_CAPABILITY_ID));
    assert!(
        failure
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("NetworkMode::Deny")
    );
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn builtin_http_rejects_hosted_allowlist_plan_before_egress() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        br#"{"ok":true}"#.to_vec(),
    ));
    let runtime = runtime_with_http_egress_and_policy(Arc::clone(&egress), hosted_dev_policy());

    let error = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Network);
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn builtin_http_defaults_json_body_content_type() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(b"ok".to_vec()));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "body": {"ok": true}
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    let requests = egress.requests();
    assert_eq!(
        requests[0].headers,
        vec![("content-type".to_string(), "application/json".to_string())]
    );
}

#[tokio::test]
async fn builtin_http_fails_closed_when_policy_allows_network_but_runtime_egress_is_missing() {
    let runtime = runtime_with_policy(local_dev_policy());
    let error = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Network);
}

#[tokio::test]
async fn builtin_http_rejects_ambiguous_body_zero_timeout_and_zero_response_limit() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let runtime = runtime_with_http_egress(egress);
    let context = execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy());

    for input in [
        json!({
            "url": "https://api.example.test/v1/items",
            "method": 123
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "body": "plain",
            "body_base64": "YmluYXJ5"
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "timeout_ms": 0
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "response_body_limit": 0
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "response_body_limit": 256 * 1024 + 1
        }),
    ] {
        let error = invoke_with_context(&runtime, HTTP_CAPABILITY_ID, input, context.clone())
            .await
            .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::InvalidInput);
    }
}

#[tokio::test]
async fn builtin_http_rejects_request_bodies_over_network_egress_cap() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let runtime = runtime_with_http_egress(Arc::clone(&egress));
    let context = execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy());
    let oversized = vec![b'a'; 256 * 1024 + 1];

    for input in [
        json!({
            "url": "https://api.example.test/v1/items",
            "body": String::from_utf8(oversized.clone()).unwrap()
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "body_base64": BASE64_STANDARD.encode(&oversized)
        }),
    ] {
        let error = invoke_with_context(&runtime, HTTP_CAPABILITY_ID, input, context.clone())
            .await
            .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::InvalidInput);
    }

    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn builtin_http_accounts_request_bytes_when_large_output_is_truncated() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(vec![
        b'\\';
        8 * 1024 * 1024
    ]));
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let runtime = runtime_with_http_egress_and_governor(Arc::clone(&egress), Arc::clone(&governor));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "method": "post",
            "url": "https://api.example.test/v1/items",
            "body": "paid",
            "response_body_limit": 256 * 1024
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    let body_text = output["body_text"].as_str().expect("text response");
    assert_eq!(body_text.len(), 128 * 1024);
    assert_eq!(output["body_bytes_returned"], json!(128 * 1024));
    assert_eq!(output["body_truncated"], json!(true));
    let tenant_account = ResourceAccount::tenant(TenantId::new(LOCAL_DEFAULT_TENANT_ID).unwrap());
    assert_eq!(governor.usage_for(&tenant_account).network_egress_bytes, 4);
}

#[tokio::test]
async fn builtin_http_returns_binary_responses_as_base64() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(vec![0, 159, 255]));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    assert_eq!(output["body_base64"], json!("AJ//"));
    assert!(output.get("body_text").is_none());
}

#[tokio::test]
async fn builtin_http_documents_mixed_utf8_response_as_binary() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        b"hello\xFFworld".to_vec(),
    ));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    // Invalid UTF-8 makes the whole response binary so callers never receive
    // split or lossy text/body fragments.
    assert_eq!(output["body_base64"], json!("aGVsbG//d29ybGQ="));
    assert!(output.get("body_text").is_none());
}

#[tokio::test]
async fn builtin_http_rejects_invalid_header_names_and_oversized_header_sets() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let runtime = runtime_with_http_egress(egress);
    let context = execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy());
    let too_many_headers = (0..65)
        .map(|index| json!({"name": format!("x-{index}"), "value": "ok"}))
        .collect::<Vec<_>>();

    for input in [
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": {"x-bad": "value\0tail"}
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": [{"name": "x\0bad", "value": "ok"}]
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": [{"name": "x bad", "value": "ok"}]
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": [{"name": "x:bad", "value": "ok"}]
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": [{"name": "x-é", "value": "ok"}]
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": too_many_headers
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": {"x-large": "a".repeat(8 * 1024 + 1)}
        }),
    ] {
        let error = invoke_with_context(&runtime, HTTP_CAPABILITY_ID, input, context.clone())
            .await
            .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::InvalidInput);
    }
}

#[tokio::test]
async fn builtin_http_maps_runtime_egress_errors_by_source() {
    let context = execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy());
    let cases = [
        (
            RuntimeHttpEgressError::Credential {
                reason: "credential missing".to_string(),
            },
            RuntimeFailureKind::Backend,
        ),
        (
            RuntimeHttpEgressError::Request {
                reason: "sensitive_header_denied:authorization".to_string(),
                request_bytes: 0,
                response_bytes: 0,
            },
            RuntimeFailureKind::InvalidInput,
        ),
        (
            RuntimeHttpEgressError::Network {
                reason: "policy_denied".to_string(),
                request_bytes: 0,
                response_bytes: 0,
            },
            RuntimeFailureKind::PolicyDenied,
        ),
        (
            RuntimeHttpEgressError::Network {
                reason: "response_body_limit_exceeded".to_string(),
                request_bytes: 4,
                response_bytes: 1024,
            },
            RuntimeFailureKind::OutputTooLarge,
        ),
        (
            RuntimeHttpEgressError::Response {
                reason: "response_body_limit_exceeded".to_string(),
                request_bytes: 4,
                response_bytes: 1024,
            },
            RuntimeFailureKind::OutputTooLarge,
        ),
        (
            RuntimeHttpEgressError::Response {
                reason: "response_decode_failed".to_string(),
                request_bytes: 4,
                response_bytes: 1024,
            },
            RuntimeFailureKind::OperationFailed,
        ),
    ];

    for (error, expected) in cases {
        let runtime =
            runtime_with_http_egress(Arc::new(RecordingRuntimeHttpEgress::with_error(error)));
        let actual = invoke_with_context(
            &runtime,
            HTTP_CAPABILITY_ID,
            json!({"url": "https://api.example.test/v1/items"}),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn builtin_http_maps_panicking_runtime_egress_to_backend_failure() {
    let runtime = runtime_with_http_egress(Arc::new(PanickingRuntimeHttpEgress));
    let error = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Backend);
}

#[tokio::test]
async fn builtin_http_rejects_sensitive_headers_through_host_validator() {
    let transport = RecordingTransport::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"ok".to_vec(),
        usage: NetworkUsage::default(),
    });
    let requests = transport.requests.clone();
    let network = PolicyNetworkHttpEgress::new_with_resolver(
        transport,
        StaticResolver::new(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]),
    );
    let runtime = runtime_with_host_http_egress(network);

    let error = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": {"authorization": "Bearer token"}
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn builtin_http_exercises_real_policy_private_ip_rejection() {
    let transport = RecordingTransport::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"ok".to_vec(),
        usage: NetworkUsage::default(),
    });
    let requests = transport.requests.clone();
    let network = PolicyNetworkHttpEgress::new_with_resolver(
        transport,
        StaticResolver::new(vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7))]),
    );
    let runtime = runtime_with_host_http_egress(network);

    let error = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::PolicyDenied);
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn builtin_http_uses_tool_call_port_for_model_visible_partial_response() {
    let transport = RecordingTransport::err(NetworkHttpError::ResponseBodyLimit {
        limit: 4,
        request_bytes: 0,
        response_bytes: 5,
        partial_response: Some(NetworkHttpResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "text/plain".to_string())],
            body: b"abcd".to_vec(),
            usage: NetworkUsage {
                request_bytes: 0,
                response_bytes: 5,
                resolved_ip: None,
            },
        }),
    });
    let requests = transport.requests.clone();
    let network = PolicyNetworkHttpEgress::new_with_resolver(
        transport,
        StaticResolver::new(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]),
    );
    let runtime = runtime_with_host_http_egress(network);

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "response_body_limit": 4
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    assert_eq!(output["status"], json!(200));
    assert_eq!(output["body_text"], json!("abcd"));
    assert_eq!(output["body_truncated"], json!(true));
    assert_eq!(output["truncation"]["body"], json!(true));
    assert_eq!(output["response_bytes"], json!(5));
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn builtin_http_returns_redirects_without_following_private_location() {
    let transport = RecordingTransport::ok(NetworkHttpResponse {
        status: 302,
        headers: vec![(
            "location".to_string(),
            "http://169.254.169.254/latest/meta-data".to_string(),
        )],
        body: Vec::new(),
        usage: NetworkUsage::default(),
    });
    let requests = transport.requests.clone();
    let network = PolicyNetworkHttpEgress::new_with_resolver(
        transport,
        StaticResolver::new(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]),
    );
    let runtime = runtime_with_host_http_egress(network);

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    assert_eq!(output["status"], json!(302));
    let recorded = requests.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].url, "https://api.example.test/v1/items");
}

#[tokio::test]
async fn builtin_http_awaits_async_egress_without_blocking_tokio_worker() {
    let egress = Arc::new(SleepingRuntimeHttpEgress {
        delay: Duration::from_millis(100),
        body: Vec::new(),
    });
    let runtime = runtime_with_http_egress(egress);
    let invocation = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    );
    tokio::pin!(invocation);

    tokio::select! {
        _ = &mut invocation => panic!("HTTP dispatch should remain pending while async egress sleeps"),
        _ = tokio::time::sleep(Duration::from_millis(20)) => {}
    }

    invocation.await.unwrap();
}

#[tokio::test]
async fn builtin_read_file_rejects_scoped_virtual_filesystem_plan_before_handler_access() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("README.md"), "must not be read\n").unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem_and_policy(filesystem, network_denied_policy());

    let error = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/README.md"}),
        execution_context_with_mounts([READ_FILE_CAPABILITY_ID], mounts),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Authorization);
}

#[tokio::test]
async fn builtin_read_file_rejects_tenant_workspace_before_filesystem_access() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("README.md"), "must not be read\n").unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem_and_policy(filesystem, hosted_dev_policy());

    let error = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/README.md"}),
        execution_context_with_mounts([READ_FILE_CAPABILITY_ID], mounts),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Authorization);
}

#[tokio::test]
async fn builtin_coding_tools_match_v1_read_write_list_glob_and_grep_shapes() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("README.md"), "alpha\nbeta\n").unwrap();
    std::fs::write(
        temp.path().join("src/lib.rs"),
        "pub fn sample() {\n    // needle\n}\n",
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts.clone());

    let read = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/README.md", "offset": 2, "limit": 1}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(read["content"], json!("     2│ beta"));
    assert_eq!(read["total_lines"], json!(2));
    assert_eq!(read["lines_shown"], json!(1));
    assert_eq!(read["truncated_by_default"], json!(false));

    let wrote = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/generated/deep.txt", "content": "created\n"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(wrote["bytes_written"], json!(8));
    assert_eq!(wrote["success"], json!(true));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("generated/deep.txt")).unwrap(),
        "created\n"
    );

    let wrote_subdir_readme = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/generated/README.md", "content": "ok\n"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(wrote_subdir_readme["success"], json!(true));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("generated/README.md")).unwrap(),
        "ok\n"
    );

    let listed = invoke_with_context(
        &runtime,
        LIST_DIR_CAPABILITY_ID,
        json!({"path": "/workspace", "recursive": true, "max_depth": 1}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(listed["count"], json!(6));
    assert_eq!(listed["truncated"], json!(false));
    assert_eq!(
        listed["entries"],
        json!([
            "generated/",
            "src/",
            "README.md (11B)",
            "generated/README.md (3B)",
            "generated/deep.txt (8B)",
            "src/lib.rs (34B)"
        ])
    );

    let globbed = invoke_with_context(
        &runtime,
        GLOB_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "**/*.rs", "max_results": 5}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(globbed["files"], json!(["src/lib.rs"]));
    assert_eq!(globbed["count"], json!(1));
    assert_eq!(globbed["truncated"], json!(false));

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "NEEDLE", "case_insensitive": true}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(grepped["files"], json!(["src/lib.rs"]));
    assert_eq!(grepped["count"], json!(1));
    assert_eq!(grepped["truncated"], json!(false));
}

#[tokio::test]
async fn builtin_coding_paths_are_relative_to_requested_root_and_zero_values_match_v1() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src/nested")).unwrap();
    std::fs::write(temp.path().join("src/lib.rs"), "needle\n").unwrap();
    std::fs::write(temp.path().join("src/nested/mod.rs"), "needle\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let read = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/src/lib.rs", "offset": 0, "limit": 0}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(read["content"], json!(""));
    assert_eq!(read["lines_shown"], json!(0));

    let listed = invoke_with_context(
        &runtime,
        LIST_DIR_CAPABILITY_ID,
        json!({"path": "/workspace/src", "recursive": true, "max_depth": 0}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(listed["entries"], json!(["nested/", "lib.rs (7B)"]));

    let globbed_empty_page = invoke_with_context(
        &runtime,
        GLOB_CAPABILITY_ID,
        json!({"path": "/workspace/src", "pattern": "*.rs", "max_results": 0}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(globbed_empty_page["files"], json!([]));
    assert_eq!(globbed_empty_page["count"], json!(0));
    assert_eq!(globbed_empty_page["truncated"], json!(true));

    let globbed = invoke_with_context(
        &runtime,
        GLOB_CAPABILITY_ID,
        json!({"path": "/workspace/src", "pattern": "*.rs"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(globbed["files"], json!(["lib.rs"]));

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace/src", "pattern": "needle"}),
        context,
    )
    .await
    .unwrap();
    let grep_files = grepped["files"].as_array().unwrap();
    assert_eq!(grep_files.len(), 2);
    assert!(
        grep_files
            .iter()
            .any(|file| file.as_str() == Some("lib.rs"))
    );
    assert!(
        grep_files
            .iter()
            .any(|file| file.as_str() == Some("nested/mod.rs"))
    );
}

#[tokio::test]
async fn builtin_coding_glob_and_grep_files_with_matches_sort_newest_first() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("old.rs"), "needle old\n").unwrap();
    thread::sleep(Duration::from_millis(1_100));
    std::fs::write(temp.path().join("new.rs"), "needle new\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let globbed = invoke_with_context(
        &runtime,
        GLOB_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "*.rs"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(globbed["files"], json!(["new.rs", "old.rs"]));

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "needle"}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(grepped["files"], json!(["new.rs", "old.rs"]));
}

#[tokio::test]
async fn builtin_coding_blocks_sensitive_scoped_paths_like_v1() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(".env"), "TOKEN=secret\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    for (capability, input) in [
        (READ_FILE_CAPABILITY_ID, json!({"path": "/workspace/.env"})),
        (
            WRITE_FILE_CAPABILITY_ID,
            json!({"path": "/workspace/.env", "content": "changed"}),
        ),
        (
            APPLY_PATCH_CAPABILITY_ID,
            json!({"path": "/workspace/.env", "old_string": "TOKEN", "new_string": "X"}),
        ),
    ] {
        let error = invoke_with_context(&runtime, capability, input, context.clone())
            .await
            .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::Authorization);
    }

    let listed = invoke_with_context(
        &runtime,
        LIST_DIR_CAPABILITY_ID,
        json!({"path": "/workspace"}),
        context,
    )
    .await
    .unwrap();
    let entries = listed["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .all(|entry| entry.as_str() != Some(".env (13B)"))
    );
}

#[tokio::test]
async fn builtin_coding_blocks_sensitive_host_paths_like_v1() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join(".ssh")).unwrap();
    std::fs::create_dir_all(temp.path().join(".aws")).unwrap();
    std::fs::create_dir_all(temp.path().join(".config/gh")).unwrap();
    std::fs::write(temp.path().join(".ssh/id_rsa"), "ssh-secret\n").unwrap();
    std::fs::write(temp.path().join(".aws/credentials"), "aws-secret\n").unwrap();
    std::fs::write(temp.path().join(".config/gh/hosts.yml"), "gh-secret\n").unwrap();
    std::fs::write(temp.path().join(".env"), "TOKEN=secret\n").unwrap();
    std::fs::write(temp.path().join("server.key"), "tls-secret\n").unwrap();
    std::fs::write(temp.path().join("safe.txt"), "safe host file\n").unwrap();

    let (filesystem, mounts) = mounted_host_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    for path in [
        "/host/.ssh/id_rsa",
        "/host/.aws/credentials",
        "/host/.config/gh/hosts.yml",
        "/host/.env",
        "/host/server.key",
    ] {
        let error = invoke_with_context(
            &runtime,
            READ_FILE_CAPABILITY_ID,
            json!({"path": path}),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::Authorization, "{path}");
    }

    for (capability, input) in [
        (
            WRITE_FILE_CAPABILITY_ID,
            json!({"path": "/host/.env", "content": "changed"}),
        ),
        (
            APPLY_PATCH_CAPABILITY_ID,
            json!({"path": "/host/server.key", "old_string": "tls", "new_string": "x"}),
        ),
    ] {
        let error = invoke_with_context(&runtime, capability, input, context.clone())
            .await
            .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::Authorization);
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(temp.path().join(".ssh"), temp.path().join("dotssh")).unwrap();
        let error = invoke_with_context(
            &runtime,
            WRITE_FILE_CAPABILITY_ID,
            json!({"path": "/host/dotssh/config", "content": "created through symlink\n"}),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::Authorization);
        assert!(
            !temp.path().join(".ssh/config").exists(),
            "write must not create files under sensitive canonical parents"
        );

        let error = invoke_with_context(
            &runtime,
            WRITE_FILE_CAPABILITY_ID,
            json!({"path": "/host/dotssh/generated/config", "content": "created through nested symlink\n"}),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::Authorization);
        assert!(
            !temp.path().join(".ssh/generated").exists(),
            "write must not create intermediate directories under sensitive canonical parents"
        );

        let error = invoke_with_context(
            &runtime,
            LIST_DIR_CAPABILITY_ID,
            json!({"path": "/host/dotssh", "recursive": true}),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::Authorization);

        let listed = invoke_with_context(
            &runtime,
            LIST_DIR_CAPABILITY_ID,
            json!({"path": "/host", "recursive": true}),
            context.clone(),
        )
        .await
        .unwrap();
        let entries = listed["entries"].as_array().unwrap();
        assert!(
            /* safety: test-only assertion. */
            entries
                .iter()
                .all(|entry| !entry.as_str().unwrap_or_default().contains("id_rsa")),
            "recursive list_dir must not traverse sensitive symlink targets: {entries:?}"
        );
    }

    let read = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/host/safe.txt"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(read["content"], json!("     1│ safe host file"));

    invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/host/output.txt", "content": "created\n"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(temp.path().join("output.txt")).unwrap(),
        "created\n"
    );

    let raw_host_home = temp.path().canonicalize().unwrap();
    let raw_host_home = raw_host_home.to_string_lossy();
    let raw_sensitive_path = format!("{raw_host_home}/.ssh/id_rsa");
    let error = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": raw_sensitive_path}),
        context.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(error, RuntimeFailureKind::Authorization);

    let raw_safe_path = format!("{raw_host_home}/safe.txt");
    let read = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": raw_safe_path}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(read["content"], json!("     1│ safe host file"));

    let raw_output_path = format!("{raw_host_home}/raw-output.txt");
    invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": raw_output_path, "content": "raw created\n"}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(temp.path().join("raw-output.txt")).unwrap(),
        "raw created\n"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn builtin_coding_blocks_sensitive_resolved_libsql_paths() {
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("filesystem.db");
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let filesystem = LibSqlRootFilesystem::new(db);
    filesystem.run_migrations().await.unwrap();
    filesystem
        .create_dir_all(&VirtualPath::new("/projects/p").unwrap())
        .await
        .unwrap();
    filesystem
        .write_file(
            &VirtualPath::new("/projects/p/.env").unwrap(),
            b"TOKEN=secret\n",
        )
        .await
        .unwrap();

    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/p/.env").unwrap(),
        MountPermissions::read_only(),
    )])
    .unwrap();
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    for (capability, input) in [
        (READ_FILE_CAPABILITY_ID, json!({"path": "/workspace"})),
        (
            GREP_CAPABILITY_ID,
            json!({"path": "/workspace", "pattern": "TOKEN"}),
        ),
    ] {
        let error = invoke_with_context(&runtime, capability, input, context.clone())
            .await
            .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::Authorization);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn builtin_coding_apply_patch_serializes_concurrent_edits_on_same_path() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("code.rs"), "A\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = std::sync::Arc::new(runtime_with_filesystem(filesystem));
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    // Prime read-before-edit state so both patches start from a valid cached hash.
    invoke_with_context(
        &*runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs"}),
        context.clone(),
    )
    .await
    .unwrap();

    let runtime_a = runtime.clone();
    let ctx_a = context.clone();
    let runtime_b = runtime.clone();
    let ctx_b = context.clone();
    let task_a = tokio::spawn(async move {
        invoke_with_context(
            &*runtime_a,
            APPLY_PATCH_CAPABILITY_ID,
            json!({"path": "/workspace/code.rs", "old_string": "A", "new_string": "X"}),
            ctx_a,
        )
        .await
    });
    let task_b = tokio::spawn(async move {
        invoke_with_context(
            &*runtime_b,
            APPLY_PATCH_CAPABILITY_ID,
            json!({"path": "/workspace/code.rs", "old_string": "A", "new_string": "Y"}),
            ctx_b,
        )
        .await
    });
    let result_a = task_a.await.unwrap();
    let result_b = task_b.await.unwrap();

    // Serialization guarantee: the second patch reads the post-write file and
    // no longer finds `old_string`, so exactly one apply_patch must succeed.
    // Without the per-path edit lock, both calls can read the original file
    // concurrently and silently lose an update.
    let outcomes = [result_a.is_ok(), result_b.is_ok()];
    assert_eq!(
        outcomes.iter().filter(|ok| **ok).count(),
        1,
        "expected exactly one concurrent apply_patch to succeed, got {:?}",
        outcomes
    );
    let final_content = std::fs::read_to_string(temp.path().join("code.rs")).unwrap();
    assert!(
        final_content == "X\n" || final_content == "Y\n",
        "unexpected final content {final_content:?}"
    );
}

#[tokio::test]
async fn builtin_coding_blocks_relative_workspace_protected_paths() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("daily")).unwrap();
    std::fs::create_dir_all(temp.path().join("context")).unwrap();
    std::fs::write(temp.path().join("README.md"), "keep\n").unwrap();
    std::fs::write(temp.path().join("daily/note.md"), "keep\n").unwrap();
    std::fs::write(temp.path().join("context/session.md"), "keep\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let root_readme_write = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "./README.md", "content": "changed\n"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(root_readme_write["success"], json!(true));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "changed\n"
    );

    for (tool_path, host_path) in [
        ("./daily/note.md", "daily/note.md"),
        ("./context/session.md", "context/session.md"),
    ] {
        let write_error = invoke_with_context(
            &runtime,
            WRITE_FILE_CAPABILITY_ID,
            json!({"path": tool_path, "content": "changed\n"}),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(write_error, RuntimeFailureKind::InvalidInput);
        assert_eq!(
            std::fs::read_to_string(temp.path().join(host_path)).unwrap(),
            "keep\n"
        );

        invoke_with_context(
            &runtime,
            READ_FILE_CAPABILITY_ID,
            json!({"path": tool_path}),
            context.clone(),
        )
        .await
        .unwrap();

        let patch_error = invoke_with_context(
            &runtime,
            APPLY_PATCH_CAPABILITY_ID,
            json!({"path": tool_path, "old_string": "keep", "new_string": "changed"}),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(patch_error, RuntimeFailureKind::InvalidInput);
        assert_eq!(
            std::fs::read_to_string(temp.path().join(host_path)).unwrap(),
            "keep\n"
        );
    }
}

#[tokio::test]
async fn builtin_coding_grep_searches_single_file_paths_like_v1() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/lib.rs"), "needle\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(
        temp.path(),
        MountPermissions {
            read: true,
            write: false,
            delete: false,
            list: false,
            execute: false,
        },
    );
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace/src/lib.rs", "pattern": "needle"}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(grepped["files"], json!(["lib.rs"]));
}

#[tokio::test]
async fn builtin_coding_grep_applies_filters_before_loading_large_files() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/lib.rs"), "needle\n").unwrap();
    std::fs::write(
        temp.path().join("huge.txt"),
        vec![b'x'; 10 * 1024 * 1024 + 1],
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "needle", "glob": "**/*.rs"}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(grepped["files"], json!(["src/lib.rs"]));
}

#[tokio::test]
async fn builtin_coding_grep_multiline_reports_matched_lines_and_count() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("doc.txt"), "alpha\nbeta\ngamma\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let content = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({
            "path": "/workspace",
            "pattern": "alpha\\nbeta",
            "output_mode": "content",
            "multiline": true
        }),
        context.clone(),
    )
    .await
    .unwrap();
    let output = content["content"].as_str().unwrap();
    assert!(output.contains("doc.txt:1:alpha"));
    assert!(output.contains("doc.txt:2:beta"));

    let counts = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({
            "path": "/workspace",
            "pattern": "alpha\\nbeta",
            "output_mode": "count",
            "multiline": true
        }),
        context,
    )
    .await
    .unwrap();
    assert_eq!(counts["total"], json!(1));
    assert_eq!(counts["counts"], json!([{ "file": "doc.txt", "count": 1 }]));
}

#[tokio::test]
async fn builtin_coding_grep_respects_multiline_false_for_line_anchors() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("doc.txt"), "alpha\nbeta\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let single_line_regex = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({
            "path": "/workspace",
            "pattern": "^beta",
            "multiline": false
        }),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(single_line_regex["files"], json!([]));

    let multiline_regex = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({
            "path": "/workspace",
            "pattern": "^beta",
            "multiline": true
        }),
        context,
    )
    .await
    .unwrap();
    assert_eq!(multiline_regex["files"], json!(["doc.txt"]));
}

#[tokio::test]
async fn builtin_coding_write_allows_v1_sized_payloads_past_default_input_cap() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);
    let content = "x".repeat(2 * 1024 * 1024);

    let wrote = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/large.txt", "content": content}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(wrote["bytes_written"], json!(2 * 1024 * 1024));
}

#[cfg(unix)]
#[tokio::test]
async fn builtin_coding_grep_skips_sensitive_files_and_symlink_targets() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(".env"), "TOKEN=secret\n").unwrap();
    std::os::unix::fs::symlink(temp.path().join(".env"), temp.path().join("safe_link.rs")).unwrap();
    std::fs::write(temp.path().join("visible.rs"), "TOKEN=visible\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "TOKEN", "output_mode": "content"}),
        context.clone(),
    )
    .await
    .unwrap();
    let content = grepped["content"].as_str().unwrap();
    assert!(content.contains("visible.rs"));
    assert!(!content.contains("secret"));
    assert!(!content.contains("safe_link"));

    let listed = invoke_with_context(
        &runtime,
        LIST_DIR_CAPABILITY_ID,
        json!({"path": "/workspace"}),
        context,
    )
    .await
    .unwrap();
    let entries = listed["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .all(|entry| !entry.as_str().unwrap_or_default().contains(".env"))
    );
    assert!(
        entries
            .iter()
            .all(|entry| !entry.as_str().unwrap_or_default().contains("safe_link"))
    );
}

#[tokio::test]
async fn builtin_coding_grep_requires_read_permission_but_glob_only_requires_list() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(
        temp.path().join("src/secret.rs"),
        "let token = \"secret\";\n",
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(
        temp.path(),
        MountPermissions {
            read: false,
            write: false,
            delete: false,
            list: true,
            execute: false,
        },
    );
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let globbed = invoke_with_context(
        &runtime,
        GLOB_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "**/*.rs"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(globbed["files"], json!(["src/secret.rs"]));

    let error = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "secret"}),
        context,
    )
    .await
    .unwrap_err();
    assert_eq!(error, RuntimeFailureKind::Authorization);
}

#[tokio::test]
async fn builtin_coding_grep_denies_directory_without_list_grant() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/lib.rs"), "needle\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(
        temp.path(),
        MountPermissions {
            read: true,
            write: false,
            delete: false,
            list: false,
            execute: false,
        },
    );
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let error = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "needle"}),
        context,
    )
    .await
    .unwrap_err();
    assert_eq!(error, RuntimeFailureKind::Authorization);
}

#[tokio::test]
async fn builtin_coding_list_and_glob_require_list_permission() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("README.md"), "alpha\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(
        temp.path(),
        MountPermissions {
            read: true,
            write: false,
            delete: false,
            list: false,
            execute: false,
        },
    );
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let read = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/README.md"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(read["content"], json!("     1│ alpha"));

    for capability in [LIST_DIR_CAPABILITY_ID, GLOB_CAPABILITY_ID] {
        let error = invoke_with_context(
            &runtime,
            capability,
            json!({"path": "/workspace", "pattern": "**/*.md"}),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::Authorization);
    }
}

#[tokio::test]
async fn builtin_coding_list_truncates_like_v1_after_500_entries() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..501 {
        std::fs::write(temp.path().join(format!("file-{index:03}.txt")), "a").unwrap();
    }

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let listed = invoke_with_context(
        &runtime,
        LIST_DIR_CAPABILITY_ID,
        json!({"path": "/workspace"}),
        execution_context_with_mounts(all_builtin_capability_ids(), mounts),
    )
    .await
    .unwrap();

    assert_eq!(listed["count"], json!(500));
    assert_eq!(listed["truncated"], json!(true));
    assert_eq!(listed["entries"].as_array().unwrap().len(), 500);
}

#[tokio::test]
async fn builtin_coding_read_rejects_files_larger_than_v1_limit_before_loading_contents() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("big.txt"),
        vec![b'x'; 10 * 1024 * 1024 + 1],
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let error = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/big.txt"}),
        execution_context_with_mounts(all_builtin_capability_ids(), mounts),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Resource);
}

#[tokio::test]
async fn builtin_coding_read_rejects_non_file_paths() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let error = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/src"}),
        execution_context_with_mounts(all_builtin_capability_ids(), mounts),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Resource);
}

#[tokio::test]
async fn builtin_apply_patch_matches_exact_unique_and_replace_all_behavior() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("code.rs"), "old\nold\nunique\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let duplicate = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs", "old_string": "old", "new_string": "new"}),
        context.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(duplicate, RuntimeFailureKind::OperationFailed);

    let replaced = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/code.rs",
            "old_string": "old",
            "new_string": "new",
            "replace_all": true
        }),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(replaced["success"], json!(true));
    assert_eq!(replaced["replacements"], json!(2));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("code.rs")).unwrap(),
        "new\nnew\nunique\n"
    );

    let no_op = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs", "old_string": "new", "new_string": "new"}),
        context,
    )
    .await
    .unwrap_err();
    assert_eq!(no_op, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn builtin_apply_patch_accepts_unique_match_without_prior_read() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("code.rs"), "old\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let patched = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs", "old_string": "old", "new_string": "new"}),
        context,
    )
    .await
    .unwrap();

    assert_eq!(patched["success"], json!(true));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("code.rs")).unwrap(),
        "new\n"
    );
}

#[tokio::test]
async fn builtin_apply_patch_accepts_file_content_written_by_same_scope() {
    let temp = tempfile::tempdir().unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/math_utils.py", "content": "def multiply(a, b):\n    return a + b\n"}),
        context.clone(),
    )
    .await
    .unwrap();

    let patched = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/math_utils.py",
            "old_string": "    return a + b",
            "new_string": "    return a * b"
        }),
        context,
    )
    .await
    .unwrap();

    assert_eq!(patched["success"], json!(true));
    assert_eq!(patched["replacements"], json!(1));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("math_utils.py")).unwrap(),
        "def multiply(a, b):\n    return a * b\n"
    );
}

#[tokio::test]
async fn builtin_apply_patch_rejects_when_old_string_is_no_longer_present() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("code.rs"), "old\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    std::fs::write(temp.path().join("code.rs"), "changed\n").unwrap();

    let missing = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs", "old_string": "old", "new_string": "new"}),
        context,
    )
    .await
    .unwrap_err();
    assert_eq!(missing, RuntimeFailureKind::OperationFailed);
}

#[tokio::test]
async fn builtin_coding_write_is_denied_by_read_only_mount() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("README.md"), "alpha\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let error = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/blocked.txt", "content": "nope"}),
        execution_context_with_mounts(all_builtin_capability_ids(), mounts),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Authorization);
    assert!(!temp.path().join("blocked.txt").exists());
}

#[tokio::test]
async fn builtin_missing_grant_denies_before_handler_dispatch() {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context(std::iter::empty::<&str>()),
            capability_id(ECHO_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({"message":"must not run"}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected authorization failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
}

async fn invoke(capability: &str, input: Value) -> Result<Value, RuntimeFailureKind> {
    let runtime = runtime();
    invoke_with_context(&runtime, capability, input, execution_context([capability])).await
}

async fn invoke_shell(input: Value) -> Result<Value, RuntimeFailureKind> {
    let runtime = runtime();
    invoke_with_context(
        &runtime,
        SHELL_CAPABILITY_ID,
        input,
        execution_context_with_network([SHELL_CAPABILITY_ID], shell_test_policy()),
    )
    .await
}

async fn invoke_with_context<R: HostRuntime + ?Sized>(
    runtime: &R,
    capability: &str,
    input: Value,
    context: ExecutionContext,
) -> Result<Value, RuntimeFailureKind> {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(capability),
            ResourceEstimate::default(),
            input,
            trust_decision(),
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => Ok(completed.output),
        RuntimeCapabilityOutcome::Failed(failure) => Err(failure.kind),
        other => panic!("unexpected capability outcome: {other:?}"),
    }
}

async fn invoke_failure_with_context<R: HostRuntime + ?Sized>(
    runtime: &R,
    capability: &str,
    input: Value,
    context: ExecutionContext,
) -> RuntimeCapabilityFailure {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(capability),
            ResourceEstimate::default(),
            input,
            trust_decision(),
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => failure,
        other => panic!("unexpected capability outcome: {other:?}"),
    }
}

fn runtime() -> impl HostRuntime {
    runtime_with_filesystem(LocalFilesystem::new())
}

fn runtime_with_filesystem<F>(filesystem: F) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
{
    runtime_with_filesystem_and_policy(filesystem, local_dev_policy())
}

fn runtime_with_filesystem_and_policy<F>(
    filesystem: F,
    policy: EffectiveRuntimePolicy,
) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
{
    runtime_with_filesystem_policy_and_trigger_repository(
        filesystem,
        policy,
        Arc::new(InMemoryTriggerRepository::default()),
    )
}

fn runtime_with_trigger_repository(repository: Arc<dyn TriggerRepository>) -> impl HostRuntime {
    runtime_with_filesystem_policy_and_trigger_repository(
        LocalFilesystem::new(),
        local_dev_policy(),
        repository,
    )
}

fn runtime_with_trigger_repository_and_create_hook(
    trigger_repository: Arc<dyn TriggerRepository>,
    trigger_create_hook: Arc<dyn TriggerCreateHook>,
) -> impl HostRuntime {
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers_with_trigger_create_hook(
            trigger_repository,
            trigger_create_hook,
        )
        .unwrap(),
    ))
    .with_runtime_http_egress(Arc::new(RecordingRuntimeHttpEgress::default()))
    .with_audit_sink(Arc::new(InMemoryAuditSink::new()))
    .with_runtime_policy(local_dev_policy())
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

#[cfg(feature = "test-support")]
fn runtime_with_trigger_repository_and_clock(
    trigger_repository: Arc<dyn TriggerRepository>,
    trigger_clock: Arc<dyn TriggerManagementClock>,
) -> impl HostRuntime {
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers_with_trigger_clock(trigger_repository, trigger_clock).unwrap(),
    ))
    .with_runtime_http_egress(Arc::new(RecordingRuntimeHttpEgress::default()))
    .with_audit_sink(Arc::new(InMemoryAuditSink::new()))
    .with_runtime_policy(local_dev_policy())
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn runtime_with_filesystem_policy_and_trigger_repository<F>(
    filesystem: F,
    policy: EffectiveRuntimePolicy,
    trigger_repository: Arc<dyn TriggerRepository>,
) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(trigger_repository).unwrap(),
    ))
    .with_runtime_http_egress(Arc::new(RecordingRuntimeHttpEgress::default()))
    .with_audit_sink(Arc::new(InMemoryAuditSink::new()))
    .with_runtime_policy(policy)
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

struct PersistedRecordTriggerCreateHook {
    repository: Arc<InMemoryTriggerRepository>,
    records: std::sync::Mutex<Vec<TriggerRecord>>,
}

impl PersistedRecordTriggerCreateHook {
    fn new(repository: Arc<InMemoryTriggerRepository>) -> Self {
        Self {
            repository,
            records: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn records(&self) -> Vec<TriggerRecord> {
        self.records.lock().unwrap().clone()
    }
}

#[async_trait]
impl TriggerCreateHook for PersistedRecordTriggerCreateHook {
    async fn after_trigger_persisted(&self, record: &TriggerRecord) -> Result<(), TriggerError> {
        let persisted = self
            .repository
            .get_trigger(record.tenant_id.clone(), record.trigger_id)
            .await
            .expect("trigger lookup succeeds");
        assert_eq!(
            persisted.as_ref().map(|record| record.trigger_id),
            Some(record.trigger_id)
        );
        self.records.lock().unwrap().push(record.clone());
        Ok(())
    }
}

#[derive(Debug)]
struct FailingTriggerCreateHook;

#[async_trait]
impl TriggerCreateHook for FailingTriggerCreateHook {
    async fn after_trigger_persisted(&self, _record: &TriggerRecord) -> Result<(), TriggerError> {
        Err(TriggerError::Backend {
            reason: "hook unavailable".to_string(),
        })
    }
}

#[derive(Debug)]
#[cfg(feature = "test-support")]
struct FixedTriggerClock(DateTime<Utc>);

#[cfg(feature = "test-support")]
impl TriggerManagementClock for FixedTriggerClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

struct FailingTriggerRepository;

#[derive(Default)]
struct RemoveFailingTriggerRepository {
    inner: InMemoryTriggerRepository,
    remove_attempts: std::sync::Mutex<usize>,
}

#[derive(Default)]
struct BatchRunHistoryFailingTriggerRepository {
    inner: InMemoryTriggerRepository,
}

impl RemoveFailingTriggerRepository {
    fn remove_attempts(&self) -> usize {
        *self.remove_attempts.lock().unwrap()
    }
}

fn trigger_backend_error() -> ironclaw_triggers::TriggerError {
    ironclaw_triggers::TriggerError::Backend {
        reason: "test backend failure".to_string(),
    }
}

#[async_trait]
impl TriggerRepository for RemoveFailingTriggerRepository {
    async fn upsert_trigger(
        &self,
        record: ironclaw_triggers::TriggerRecord,
    ) -> Result<(), ironclaw_triggers::TriggerError> {
        self.inner.upsert_trigger(record).await
    }

    async fn get_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: ironclaw_triggers::TriggerId,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.get_trigger(tenant_id, trigger_id).await
    }

    async fn list_triggers(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.list_triggers(tenant_id).await
    }

    async fn list_scoped_triggers(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        limit: usize,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner
            .list_scoped_triggers(tenant_id, creator_user_id, agent_id, project_id, limit)
            .await
    }

    async fn remove_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: ironclaw_triggers::TriggerId,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        *self.remove_attempts.lock().unwrap() += 1;
        Err(trigger_backend_error())
    }

    async fn remove_scoped_trigger(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        trigger_id: ironclaw_triggers::TriggerId,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner
            .remove_scoped_trigger(tenant_id, creator_user_id, agent_id, project_id, trigger_id)
            .await
    }

    async fn list_due_triggers(
        &self,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.list_due_triggers(now, limit).await
    }

    async fn list_active_triggers(
        &self,
        limit: usize,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.list_active_triggers(limit).await
    }

    async fn list_active_triggers_after(
        &self,
        after: Option<ironclaw_triggers::ActiveTriggerScanCursor>,
        limit: usize,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.list_active_triggers_after(after, limit).await
    }

    async fn claim_due_fire(
        &self,
        request: ironclaw_triggers::ClaimDueFireRequest,
    ) -> Result<ironclaw_triggers::ClaimDueFireOutcome, ironclaw_triggers::TriggerError> {
        self.inner.claim_due_fire(request).await
    }

    async fn mark_fire_accepted(
        &self,
        request: ironclaw_triggers::FireAcceptedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.mark_fire_accepted(request).await
    }

    async fn mark_fire_replayed(
        &self,
        request: ironclaw_triggers::FireReplayedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.mark_fire_replayed(request).await
    }

    async fn mark_fire_retryable_failed(
        &self,
        request: ironclaw_triggers::FireRetryableFailedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.mark_fire_retryable_failed(request).await
    }

    async fn mark_fire_permanently_failed(
        &self,
        request: ironclaw_triggers::FirePermanentFailedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.mark_fire_permanently_failed(request).await
    }

    async fn mark_fire_terminally_failed(
        &self,
        request: ironclaw_triggers::FireTerminalFailedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.mark_fire_terminally_failed(request).await
    }

    async fn clear_active_fire(
        &self,
        request: ironclaw_triggers::ClearActiveFireRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.clear_active_fire(request).await
    }
}

#[async_trait]
impl TriggerRepository for BatchRunHistoryFailingTriggerRepository {
    async fn upsert_trigger(
        &self,
        record: ironclaw_triggers::TriggerRecord,
    ) -> Result<(), ironclaw_triggers::TriggerError> {
        self.inner.upsert_trigger(record).await
    }

    async fn get_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: ironclaw_triggers::TriggerId,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.get_trigger(tenant_id, trigger_id).await
    }

    async fn list_triggers(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.list_triggers(tenant_id).await
    }

    async fn list_scoped_triggers(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        limit: usize,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner
            .list_scoped_triggers(tenant_id, creator_user_id, agent_id, project_id, limit)
            .await
    }

    async fn remove_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: ironclaw_triggers::TriggerId,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.remove_trigger(tenant_id, trigger_id).await
    }

    async fn remove_scoped_trigger(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        trigger_id: ironclaw_triggers::TriggerId,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner
            .remove_scoped_trigger(tenant_id, creator_user_id, agent_id, project_id, trigger_id)
            .await
    }

    async fn list_due_triggers(
        &self,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.list_due_triggers(now, limit).await
    }

    async fn list_active_triggers(
        &self,
        limit: usize,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.list_active_triggers(limit).await
    }

    async fn list_active_triggers_after(
        &self,
        after: Option<ironclaw_triggers::ActiveTriggerScanCursor>,
        limit: usize,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.list_active_triggers_after(after, limit).await
    }

    async fn claim_due_fire(
        &self,
        request: ironclaw_triggers::ClaimDueFireRequest,
    ) -> Result<ironclaw_triggers::ClaimDueFireOutcome, ironclaw_triggers::TriggerError> {
        self.inner.claim_due_fire(request).await
    }

    async fn mark_fire_accepted(
        &self,
        request: ironclaw_triggers::FireAcceptedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.mark_fire_accepted(request).await
    }

    async fn mark_fire_replayed(
        &self,
        request: ironclaw_triggers::FireReplayedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.mark_fire_replayed(request).await
    }

    async fn mark_fire_retryable_failed(
        &self,
        request: ironclaw_triggers::FireRetryableFailedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.mark_fire_retryable_failed(request).await
    }

    async fn mark_fire_permanently_failed(
        &self,
        request: ironclaw_triggers::FirePermanentFailedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.mark_fire_permanently_failed(request).await
    }

    async fn mark_fire_terminally_failed(
        &self,
        request: ironclaw_triggers::FireTerminalFailedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.mark_fire_terminally_failed(request).await
    }

    async fn clear_active_fire(
        &self,
        request: ironclaw_triggers::ClearActiveFireRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        self.inner.clear_active_fire(request).await
    }

    async fn list_trigger_run_history_batch(
        &self,
        _tenant_id: TenantId,
        _trigger_ids: &[ironclaw_triggers::TriggerId],
        _limit: usize,
    ) -> Result<HashMap<ironclaw_triggers::TriggerId, Vec<TriggerRunRecord>>, TriggerError> {
        Err(trigger_backend_error())
    }
}

#[async_trait]
impl TriggerRepository for FailingTriggerRepository {
    async fn upsert_trigger(
        &self,
        _record: ironclaw_triggers::TriggerRecord,
    ) -> Result<(), ironclaw_triggers::TriggerError> {
        Err(trigger_backend_error())
    }

    async fn get_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: ironclaw_triggers::TriggerId,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        Err(trigger_backend_error())
    }

    async fn list_triggers(
        &self,
        _tenant_id: TenantId,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        Err(trigger_backend_error())
    }

    async fn list_scoped_triggers(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _limit: usize,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        Err(trigger_backend_error())
    }

    async fn remove_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: ironclaw_triggers::TriggerId,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        Err(trigger_backend_error())
    }

    async fn remove_scoped_trigger(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _trigger_id: ironclaw_triggers::TriggerId,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        Err(trigger_backend_error())
    }

    async fn list_due_triggers(
        &self,
        _now: Timestamp,
        _limit: usize,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        unreachable!("failing test repository does not list due triggers")
    }

    async fn list_active_triggers(
        &self,
        _limit: usize,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        unreachable!("failing test repository does not list active triggers")
    }

    async fn list_active_triggers_after(
        &self,
        _after: Option<ironclaw_triggers::ActiveTriggerScanCursor>,
        _limit: usize,
    ) -> Result<Vec<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        unreachable!("failing test repository does not list active triggers")
    }

    async fn claim_due_fire(
        &self,
        _request: ironclaw_triggers::ClaimDueFireRequest,
    ) -> Result<ironclaw_triggers::ClaimDueFireOutcome, ironclaw_triggers::TriggerError> {
        unreachable!("failing test repository does not claim due fires")
    }

    async fn mark_fire_accepted(
        &self,
        _request: ironclaw_triggers::FireAcceptedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        unreachable!("failing test repository does not update fire results")
    }

    async fn mark_fire_replayed(
        &self,
        _request: ironclaw_triggers::FireReplayedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        unreachable!("failing test repository does not update fire results")
    }

    async fn mark_fire_retryable_failed(
        &self,
        _request: ironclaw_triggers::FireRetryableFailedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        unreachable!("failing test repository does not update fire results")
    }

    async fn mark_fire_permanently_failed(
        &self,
        _request: ironclaw_triggers::FirePermanentFailedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        unreachable!("failing test repository does not update fire results")
    }

    async fn mark_fire_terminally_failed(
        &self,
        _request: ironclaw_triggers::FireTerminalFailedRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        unreachable!("failing test repository does not update fire results")
    }

    async fn clear_active_fire(
        &self,
        _request: ironclaw_triggers::ClearActiveFireRequest,
    ) -> Result<Option<ironclaw_triggers::TriggerRecord>, ironclaw_triggers::TriggerError> {
        unreachable!("failing test repository does not clear active fires")
    }
}

fn runtime_with_filesystem_without_http_egress<F>(filesystem: F) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_policy(local_dev_policy())
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn runtime_with_filesystem_and_http_egress<F, T>(filesystem: F, egress: Arc<T>) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
    T: RuntimeHttpEgress + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_http_egress(egress)
    .with_runtime_policy(local_dev_policy())
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn runtime_with_process_port<T>(process_port: Arc<T>) -> impl HostRuntime
where
    T: RuntimeProcessPort + 'static,
{
    runtime_with_process_port_and_policy(process_port, local_dev_policy())
}

fn runtime_with_process_port_and_policy<T>(
    process_port: Arc<T>,
    policy: EffectiveRuntimePolicy,
) -> impl HostRuntime
where
    T: RuntimeProcessPort + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_process_port(process_port)
    .with_runtime_http_egress(Arc::new(RecordingRuntimeHttpEgress::default()))
    .with_runtime_policy(policy)
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn runtime_with_local_and_sandbox_process_ports<L>(
    local_process: Arc<L>,
    sandbox_process: Arc<TenantSandboxProcessPort>,
    policy: EffectiveRuntimePolicy,
) -> impl HostRuntime
where
    L: RuntimeProcessPort + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_process_port(local_process)
    .with_tenant_sandbox_process_port(sandbox_process)
    .with_runtime_http_egress(Arc::new(RecordingRuntimeHttpEgress::default()))
    .with_runtime_policy(policy)
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn runtime_with_policy(policy: EffectiveRuntimePolicy) -> impl HostRuntime {
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_trust_policy(Arc::new(trust_policy()))
    .with_runtime_policy(policy)
    .host_runtime_for_local_testing()
}

fn local_dev_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

fn tenant_sandbox_process_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        process_backend: ProcessBackendKind::TenantSandbox,
        ..local_dev_policy()
    }
}

fn hosted_dev_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::HostedMultiTenant,
        requested_profile: RuntimeProfile::HostedDev,
        resolved_profile: RuntimeProfile::HostedDev,
        filesystem_backend: FilesystemBackendKind::TenantWorkspace,
        process_backend: ProcessBackendKind::TenantSandbox,
        network_mode: NetworkMode::Allowlist,
        secret_mode: SecretMode::TenantBroker,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::Standard,
    }
}

fn runtime_with_http_egress<T>(egress: Arc<T>) -> impl HostRuntime
where
    T: RuntimeHttpEgress + ToolCallHttpEgress + 'static,
{
    runtime_with_http_egress_and_governor(egress, Arc::new(InMemoryResourceGovernor::new()))
}

fn runtime_with_http_egress_and_policy<T>(
    egress: Arc<T>,
    policy: EffectiveRuntimePolicy,
) -> impl HostRuntime
where
    T: RuntimeHttpEgress + ToolCallHttpEgress + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_first_party_http_egress(egress)
    .with_trust_policy(Arc::new(trust_policy()))
    .with_runtime_policy(policy)
    .host_runtime_for_local_testing()
}

fn runtime_with_http_egress_and_governor<T>(
    egress: Arc<T>,
    governor: Arc<InMemoryResourceGovernor>,
) -> impl HostRuntime
where
    T: RuntimeHttpEgress + ToolCallHttpEgress + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        governor,
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_first_party_http_egress(egress)
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn runtime_with_strict_http_egress_only<T>(egress: Arc<T>) -> impl HostRuntime
where
    T: RuntimeHttpEgress + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_http_egress(egress)
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn runtime_with_host_http_egress<N>(network: N) -> impl HostRuntime
where
    N: NetworkHttpEgress + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::new(InMemorySecretStore::new()))
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .try_with_host_http_egress(network)
    .unwrap()
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn registry() -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(builtin_first_party_package().unwrap())
        .unwrap();
    registry
}

fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

fn provider_id() -> ExtensionId {
    ExtensionId::new("builtin").unwrap()
}

fn all_builtin_capability_ids() -> Vec<&'static str> {
    vec![
        ECHO_CAPABILITY_ID,
        TIME_CAPABILITY_ID,
        JSON_CAPABILITY_ID,
        HTTP_CAPABILITY_ID,
        HTTP_SAVE_CAPABILITY_ID,
        SHELL_CAPABILITY_ID,
        SPAWN_SUBAGENT_CAPABILITY_ID,
        MEMORY_SEARCH_CAPABILITY_ID,
        MEMORY_WRITE_CAPABILITY_ID,
        MEMORY_READ_CAPABILITY_ID,
        MEMORY_TREE_CAPABILITY_ID,
        READ_FILE_CAPABILITY_ID,
        WRITE_FILE_CAPABILITY_ID,
        LIST_DIR_CAPABILITY_ID,
        GLOB_CAPABILITY_ID,
        GREP_CAPABILITY_ID,
        APPLY_PATCH_CAPABILITY_ID,
        SKILL_LIST_CAPABILITY_ID,
        SKILL_INSTALL_CAPABILITY_ID,
        SKILL_REMOVE_CAPABILITY_ID,
        TRIGGER_CREATE_CAPABILITY_ID,
        TRIGGER_LIST_CAPABILITY_ID,
        TRIGGER_REMOVE_CAPABILITY_ID,
    ]
}

fn mounted_filesystem(path: &Path, permissions: MountPermissions) -> (LocalFilesystem, MountView) {
    let mut filesystem = LocalFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/projects/coding-pack").unwrap(),
            HostPath::from_path_buf(path.to_path_buf()),
        )
        .unwrap();
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/coding-pack").unwrap(),
        permissions,
    )])
    .unwrap();
    (filesystem, mounts)
}

fn in_memory_mounted_filesystem(permissions: MountPermissions) -> (InMemoryBackend, MountView) {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/coding-pack").unwrap(),
        permissions,
    )])
    .unwrap();
    (InMemoryBackend::new(), mounts)
}

fn memory_mounts(permissions: MountPermissions) -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/memory").unwrap(),
        VirtualPath::new("/memory").unwrap(),
        permissions,
    )])
    .unwrap()
}

fn mounted_skill_filesystem(path: &Path) -> (LocalFilesystem, MountView) {
    let mut filesystem = LocalFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/projects/skills").unwrap(),
            HostPath::from_path_buf(path.to_path_buf()),
        )
        .unwrap();
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/skills").unwrap(),
        VirtualPath::new("/projects/skills").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    (filesystem, mounts)
}

fn mounted_host_filesystem(
    path: &Path,
    permissions: MountPermissions,
) -> (LocalFilesystem, MountView) {
    let mut filesystem = LocalFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/projects/host").unwrap(),
            HostPath::from_path_buf(path.to_path_buf()),
        )
        .unwrap();
    let raw_alias = path.canonicalize().unwrap().to_string_lossy().into_owned();
    let mounts = MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/host").unwrap(),
            VirtualPath::new("/projects/host").unwrap(),
            permissions.clone(),
        ),
        MountGrant::new(
            MountAlias::new(raw_alias).unwrap(),
            VirtualPath::new("/projects/host").unwrap(),
            permissions,
        ),
    ])
    .unwrap();
    (filesystem, mounts)
}

fn skill_bundle_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    skill_bundle_zip_owned(
        files
            .iter()
            .map(|(path, content)| ((*path).to_string(), (*content).to_vec())),
    )
}

fn skill_bundle_zip_owned(files: impl IntoIterator<Item = (String, Vec<u8>)>) -> Vec<u8> {
    skill_bundle_zip_with_dirs(std::iter::empty::<String>(), files)
}

fn skill_bundle_zip_with_dirs<D>(
    directories: impl IntoIterator<Item = D>,
    files: impl IntoIterator<Item = (String, Vec<u8>)>,
) -> Vec<u8>
where
    D: AsRef<str>,
{
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    let options = zip::write::SimpleFileOptions::default();
    for path in directories {
        writer.add_directory(path.as_ref(), options).unwrap();
    }
    for (path, content) in files {
        writer.start_file(path, options).unwrap();
        writer.write_all(&content).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[derive(Debug, Default)]
struct RecordingProcessPort {
    requests: std::sync::Mutex<Vec<CommandExecutionRequest>>,
}

#[async_trait]
impl RuntimeProcessPort for RecordingProcessPort {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(CommandExecutionOutput {
            output: format!("process port: {}", request.command),
            saved_output: None,
            exit_code: 0,
            sandboxed: true,
            duration: Duration::from_millis(7),
        })
    }
}

#[derive(Debug, Default)]
struct RecordingSandboxTransport {
    requests: std::sync::Mutex<Vec<CommandExecutionRequest>>,
}

#[async_trait]
impl SandboxCommandTransport for RecordingSandboxTransport {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(CommandExecutionOutput {
            output: format!("process port: {}", request.command),
            saved_output: None,
            exit_code: 0,
            sandboxed: false,
            duration: Duration::from_millis(7),
        })
    }
}

#[derive(Debug, Clone, Default)]
struct RecordingRuntimeHttpEgress {
    requests: Arc<std::sync::Mutex<Vec<RuntimeHttpEgressRequest>>>,
    status: u16,
    body: Vec<u8>,
    saved_body: Option<RuntimeHttpSavedBody>,
    error: Option<RuntimeHttpEgressError>,
    responses: Option<BTreeMap<String, (u16, Vec<u8>)>>,
    headers: Option<Vec<(String, String)>>,
    redaction_applied: bool,
    response_bytes_override: Option<u64>,
}

impl RecordingRuntimeHttpEgress {
    fn with_body(body: Vec<u8>) -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
            status: 200,
            body,
            saved_body: None,
            error: None,
            responses: None,
            headers: None,
            redaction_applied: false,
            response_bytes_override: None,
        }
    }

    fn with_status_and_body(status: u16, body: Vec<u8>) -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
            status,
            body,
            saved_body: None,
            error: None,
            responses: None,
            headers: None,
            redaction_applied: false,
            response_bytes_override: None,
        }
    }

    fn with_error(error: RuntimeHttpEgressError) -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
            status: 200,
            body: Vec::new(),
            saved_body: None,
            error: Some(error),
            responses: None,
            headers: None,
            redaction_applied: false,
            response_bytes_override: None,
        }
    }

    fn with_saved_body(mut self, path: &str, bytes_written: u64) -> Self {
        self.saved_body = Some(RuntimeHttpSavedBody {
            path: ScopedPath::new(path).unwrap(),
            bytes_written,
        });
        self
    }

    fn with_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.headers = Some(headers);
        self
    }

    fn with_redaction_applied(mut self, redaction_applied: bool) -> Self {
        self.redaction_applied = redaction_applied;
        self
    }

    fn with_response_bytes(mut self, response_bytes: u64) -> Self {
        self.response_bytes_override = Some(response_bytes);
        self
    }

    fn with_url_bodies(responses: BTreeMap<String, (u16, Vec<u8>)>) -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
            status: 200,
            body: Vec::new(),
            saved_body: None,
            error: None,
            responses: Some(responses),
            headers: None,
            redaction_applied: false,
            response_bytes_override: None,
        }
    }

    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for RecordingRuntimeHttpEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests.lock().unwrap().push(request.clone());
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let (status, body) = self
            .responses
            .as_ref()
            .and_then(|responses| responses.get(&request.url).cloned())
            .unwrap_or_else(|| {
                (
                    if self.status == 0 { 200 } else { self.status },
                    self.body.clone(),
                )
            });
        Ok(RuntimeHttpEgressResponse {
            status,
            headers: self.headers.clone().unwrap_or_else(|| {
                vec![("content-type".to_string(), "application/json".to_string())]
            }),
            body: body.clone(),
            saved_body: self.saved_body.clone(),
            request_bytes: request.body.len() as u64,
            response_bytes: self.response_bytes_override.unwrap_or(body.len() as u64),
            redaction_applied: self.redaction_applied,
        })
    }
}

#[async_trait::async_trait]
impl ToolCallHttpEgress for RecordingRuntimeHttpEgress {
    async fn execute_for_model_visible_output(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.execute(request).await
    }
}

fn serialized_json_len(value: &Value) -> usize {
    serde_json::to_vec(value).unwrap().len()
}

#[derive(Debug, Clone)]
struct SleepingRuntimeHttpEgress {
    delay: Duration,
    body: Vec<u8>,
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for SleepingRuntimeHttpEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        tokio::time::sleep(self.delay).await;
        let body = if self.body.is_empty() {
            b"ok".to_vec()
        } else {
            self.body.clone()
        };
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            response_bytes: body.len() as u64,
            body,
            saved_body: None,
            request_bytes: request.body.len() as u64,
            redaction_applied: false,
        })
    }
}

#[async_trait::async_trait]
impl ToolCallHttpEgress for SleepingRuntimeHttpEgress {
    async fn execute_for_model_visible_output(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.execute(request).await
    }
}

#[derive(Debug, Clone)]
struct PanickingRuntimeHttpEgress;

#[async_trait::async_trait]
impl RuntimeHttpEgress for PanickingRuntimeHttpEgress {
    async fn execute(
        &self,
        _request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        panic!("runtime HTTP egress panic")
    }
}

#[async_trait::async_trait]
impl ToolCallHttpEgress for PanickingRuntimeHttpEgress {
    async fn execute_for_model_visible_output(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.execute(request).await
    }
}

#[derive(Debug, Clone)]
struct PanickingToolCallHttpEgress;

#[async_trait::async_trait]
impl ToolCallHttpEgress for PanickingToolCallHttpEgress {
    async fn execute_for_model_visible_output(
        &self,
        _request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        panic!("tool-call HTTP egress must not be used")
    }
}

#[derive(Debug, Clone)]
struct RecordingTransport {
    response: Result<NetworkHttpResponse, NetworkHttpError>,
    requests: Arc<std::sync::Mutex<Vec<NetworkTransportRequest>>>,
}

impl RecordingTransport {
    fn ok(response: NetworkHttpResponse) -> Self {
        Self {
            response: Ok(response),
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn err(error: NetworkHttpError) -> Self {
        Self {
            response: Err(error),
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl NetworkHttpTransport for RecordingTransport {
    async fn execute(
        &self,
        request: NetworkTransportRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        self.requests.lock().unwrap().push(request);
        self.response.clone()
    }
}

#[derive(Debug, Clone)]
struct StaticResolver {
    ips: Vec<IpAddr>,
}

impl StaticResolver {
    fn new(ips: Vec<IpAddr>) -> Self {
        Self { ips }
    }
}

impl NetworkResolver for StaticResolver {
    fn resolve_ips(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>, NetworkHttpError> {
        Ok(self.ips.clone())
    }
}

fn execution_context<I>(grants: I) -> ExecutionContext
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let capability_set = CapabilitySet {
        grants: grants
            .into_iter()
            .map(|grant| dispatch_grant(grant.as_ref()))
            .collect(),
    };
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        capability_set,
        MountView::default(),
    )
    .unwrap()
}

fn execution_context_with_mounts<I>(grants: I, mounts: MountView) -> ExecutionContext
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let capability_set = CapabilitySet {
        grants: grants
            .into_iter()
            .map(|grant| dispatch_grant_with_mounts(grant.as_ref(), mounts.clone()))
            .collect(),
    };
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        capability_set,
        mounts,
    )
    .unwrap()
}

fn execution_context_with_network<I>(grants: I, network: NetworkPolicy) -> ExecutionContext
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    execution_context_with_mounts_and_network(grants, MountView::default(), network)
}

fn execution_context_with_mounts_and_network<I>(
    grants: I,
    mounts: MountView,
    network: NetworkPolicy,
) -> ExecutionContext
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let capability_set = CapabilitySet {
        grants: grants
            .into_iter()
            .map(|grant| {
                dispatch_grant_with_mounts_and_network(
                    grant.as_ref(),
                    mounts.clone(),
                    network.clone(),
                )
            })
            .collect(),
    };
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        capability_set,
        mounts,
    )
    .unwrap()
}

fn set_context_scope(
    context: &mut ExecutionContext,
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
) {
    context.tenant_id = tenant_id.clone();
    context.user_id = user_id.clone();
    context.agent_id = agent_id.clone();
    context.project_id = project_id.clone();
    context.resource_scope.tenant_id = tenant_id;
    context.resource_scope.user_id = user_id;
    context.resource_scope.agent_id = agent_id;
    context.resource_scope.project_id = project_id;
}

fn dispatch_grant(capability: &str) -> CapabilityGrant {
    dispatch_grant_with_mounts(capability, MountView::default())
}

fn dispatch_grant_with_mounts(capability: &str, mounts: MountView) -> CapabilityGrant {
    dispatch_grant_with_mounts_and_network(capability, mounts, NetworkPolicy::default())
}

fn dispatch_grant_with_mounts_and_network(
    capability: &str,
    mounts: MountView,
    network: NetworkPolicy,
) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(capability),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: builtin_effects(),
            mounts,
            network,
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn builtin_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
        EffectKind::DeleteFilesystem,
        EffectKind::Network,
        EffectKind::SpawnProcess,
        EffectKind::ExecuteCode,
        EffectKind::ExternalWrite,
    ]
}

fn network_denied_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::SecureDefault,
        resolved_profile: RuntimeProfile::SecureDefault,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::None,
        network_mode: NetworkMode::Deny,
        secret_mode: SecretMode::BrokeredHandles,
        approval_policy: ApprovalPolicy::AskAlways,
        audit_mode: AuditMode::LocalMinimal,
    }
}

fn http_test_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
}

fn shell_test_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: None,
            host_pattern: "*".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: false,
        max_egress_bytes: None,
    }
}

fn trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("builtin").unwrap(),
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            builtin_effects(),
            None,
        ),
    ]))])
    .unwrap()
}

fn provider_trust() -> BTreeMap<ExtensionId, TrustDecision> {
    BTreeMap::from([(provider_id(), trust_decision())])
}

fn trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: builtin_effects(),
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: chrono::Utc::now(),
    }
}
