//! `qa_smoke` domain tools profile. `qa_smoke_tools()` builds the host
//! runtime directly (rather than through `new_with_options`) so it can wire a
//! scripted `RecordingRuntimeHttpEgress` body at construction time — a bespoke
//! full constructor rather than a `ToolsProfile`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ironclaw_host_api::{
    CapabilityId, EffectKind, ExtensionId, MountPermissions, RuntimeKind, UserId,
};
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, BUILTIN_FIRST_PARTY_PROVIDER, ECHO_CAPABILITY_ID,
    GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, HTTP_CAPABILITY_ID, HTTP_SAVE_CAPABILITY_ID,
    JSON_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID, MEMORY_READ_CAPABILITY_ID,
    MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID,
    READ_FILE_CAPABILITY_ID, SHELL_CAPABILITY_ID, SKILL_INSTALL_CAPABILITY_ID,
    SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID, SPAWN_SUBAGENT_CAPABILITY_ID,
    TIME_CAPABILITY_ID, TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID,
    TRIGGER_PAUSE_CAPABILITY_ID, TRIGGER_REMOVE_CAPABILITY_ID, TRIGGER_RESUME_CAPABILITY_ID,
    WRITE_FILE_CAPABILITY_ID,
};
use ironclaw_reborn_composition::ProductLiveCapabilityIo;

use super::super::{
    HarnessResult, HostRuntimeCapabilityHarness, RecordingRuntimeHttpEgress,
    host_runtime_storage_roots, http_test_policy, local_dev_host_runtime_with_http_egress,
    memory_mounts, qa_smoke_mounts,
};

/// Real capability ids `qa_smoke_tools` registers on the built harness — a
/// single source shared with the wiring-parity capability-id subset check
/// (`tests/integration/wiring_parity.rs`) instead of a second
/// hand-transcribed copy of this list.
pub(crate) fn qa_smoke_tools_capability_ids() -> HarnessResult<Vec<CapabilityId>> {
    Ok(vec![
        CapabilityId::new(ECHO_CAPABILITY_ID)?,
        CapabilityId::new(TIME_CAPABILITY_ID)?,
        CapabilityId::new(JSON_CAPABILITY_ID)?,
        CapabilityId::new(HTTP_CAPABILITY_ID)?,
        CapabilityId::new(HTTP_SAVE_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_READ_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_TREE_CAPABILITY_ID)?,
        CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
        CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?,
        CapabilityId::new(LIST_DIR_CAPABILITY_ID)?,
        CapabilityId::new(GLOB_CAPABILITY_ID)?,
        CapabilityId::new(GREP_CAPABILITY_ID)?,
        CapabilityId::new(APPLY_PATCH_CAPABILITY_ID)?,
        CapabilityId::new(SHELL_CAPABILITY_ID)?,
        CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID)?,
        CapabilityId::new(SKILL_LIST_CAPABILITY_ID)?,
        CapabilityId::new(SKILL_INSTALL_CAPABILITY_ID)?,
        CapabilityId::new(SKILL_REMOVE_CAPABILITY_ID)?,
        CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID)?,
        CapabilityId::new(TRIGGER_LIST_CAPABILITY_ID)?,
        CapabilityId::new(TRIGGER_PAUSE_CAPABILITY_ID)?,
        CapabilityId::new(TRIGGER_RESUME_CAPABILITY_ID)?,
        CapabilityId::new(TRIGGER_REMOVE_CAPABILITY_ID)?,
    ])
}

pub(crate) async fn qa_smoke_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
    std::fs::create_dir_all(storage_root.join("skills"))?;
    std::fs::create_dir_all(storage_root.join("system/skills"))?;
    let runtime = local_dev_host_runtime_with_http_egress(
        storage_root,
        Arc::new(RecordingRuntimeHttpEgress::with_body(
            br#"{"accepted":true,"source":"qa-smoke"}"#.to_vec(),
        )),
        // qa_smoke_tools exercises real process execution (SpawnProcess effect);
        // leave the default LocalHostProcessPort in place.
        None,
    )?;
    let mounts = qa_smoke_mounts()?;
    let memory_mounts = memory_mounts(MountPermissions::read_write_list_delete())?;
    let memory_capability_ids = [
        CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_READ_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_TREE_CAPABILITY_ID)?,
    ];
    Ok(HostRuntimeCapabilityHarness {
        runtime,
        approval_parts: None,
        auto_approve_settings: None,
        pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
        io: Arc::new(ProductLiveCapabilityIo::default()),
        root,
        workspace_root,
        mounts,
        capability_mount_overrides: memory_capability_ids
            .iter()
            .cloned()
            .map(|capability_id| (capability_id, memory_mounts.clone()))
            .collect(),
        capability_ids: qa_smoke_tools_capability_ids()?,
        runtime_kind: RuntimeKind::FirstParty,
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::DeleteFilesystem,
            EffectKind::Network,
            EffectKind::SpawnProcess,
            EffectKind::ExecuteCode,
            EffectKind::ExternalWrite,
        ],
        network_policy: http_test_policy(),
        secrets: Vec::new(),
        provider_id: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
        additional_provider_trust: Vec::new(),
        user_id: UserId::new("reborn-e2e-qa-smoke-user")?,
        invocations: Arc::new(Mutex::new(Vec::new())),
        results: Arc::new(Mutex::new(Vec::new())),
        http_egress: None,
        network_egress: None,
        process_port: None,
        profile_filesystem: None,
        project_service: None,
        skill_activation_source: None,
        attachment_test_support: None,
        inbound_attachment_reader: None,
        outbound_target_tools: None,
        scope_capability_by_run_owner: false,
        product_auth: None,
        tool_permission_overrides: None,
        persistent_approval_policies: None,
        trigger_repository: None,
    })
}
