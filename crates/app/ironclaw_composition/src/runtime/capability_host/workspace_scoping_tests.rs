//! Caller-level tests for where an agent's workspace *writes* land.
//!
//! The production seam is `RefreshingLoopCapabilityPortFactory::create_capability_port`:
//! it mints the `mounts = "workspace"` grants every file tool resolves paths
//! through. On a hosted (multi-user) deployment those grants must point at the
//! caller's own `tenants/{tenant}/users/{user}` subtree — the same subtree the
//! WebUI workspace browser reads — and on a standalone deployment they must keep
//! pointing at the shared workspace root.

use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    ids::{AgentId, CapabilityId, ProjectId, ProviderToolName, TenantId, ThreadId, UserId},
    path::VirtualPath,
    resolution::Resolution,
};
use ironclaw_host_runtime::{CODING_READ_CAPABILITY_ID, CODING_WRITE_CAPABILITY_ID};
use ironclaw_loop_contracts::{
    InMemoryLoopHostMilestoneSink, InMemoryRunProfileResolver, LoopRequest, LoopRunContext,
    ProviderToolCall, RunProfileResolutionRequest, RunProfileResolver, VisibleCapabilityRequest,
};
use ironclaw_loop_host::{
    LoopCapabilityInputResolver, LoopCapabilityPortFactory, LoopCapabilityResultWriter,
};
use ironclaw_turns::{TurnId, TurnRunId, TurnScope};

use super::{
    ExtensionCapabilitySurfaceSource, RefreshingLoopCapabilityPortFactory, StagedCapabilityIo,
};
use crate::RebornCompositionProfile;
use crate::factory::RebornRuntimeStores;

const TENANT: &str = "workspace-scoping-tenant";

/// Write `/workspace/{file_name}` through the production capability port for
/// `user_id`, then return the composed runtime so the caller can assert where
/// the bytes physically landed.
async fn write_workspace_file_as(
    services: &RebornRuntimeStores,
    user_id: &str,
    file_name: &str,
    body: &str,
) {
    let outcome = invoke_workspace_tool_as(
        services,
        user_id,
        "builtin_write",
        CODING_WRITE_CAPABILITY_ID,
        serde_json::json!({
            "path": format!("/workspace/{file_name}"),
            "content": body,
        }),
    )
    .await;
    assert_tool_succeeded(&outcome, "write");
}

/// Drive one first-party workspace tool through the production capability port
/// as `user_id`, returning the tool's own JSON output.
///
/// The port is the seam that mints the caller's workspace grant, so every
/// assertion made on the result reflects the mount view a real run would get.
async fn invoke_workspace_tool_as(
    services: &RebornRuntimeStores,
    user_id: &str,
    provider_tool_name: &str,
    capability_id: &str,
    arguments: serde_json::Value,
) -> ToolOutcome {
    let runtime_surfaces = services
        .local_runtime_for_test()
        .expect("local runtime substrate"); // safety: test-only assertion in #[cfg(test)] module.
    let user = UserId::new(user_id).expect("user id"); // safety: test-only literal id.
    let run_context = run_context_for(user_id, &user).await;
    enable_global_auto_approve(services, &run_context, &user).await;

    let capability_io = Arc::new(StagedCapabilityIo::default());
    let input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
    let result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
    let factory = RefreshingLoopCapabilityPortFactory {
        runtime: services.host_runtime.clone(),
        fallback_user_id: UserId::new("workspace-scoping-fallback").expect("user id"), // safety: test-only literal id.
        policy: Arc::new(
            crate::builtin_capability_policy::builtin_capability_policy().expect("policy parses"), // safety: test-only assertion in #[cfg(test)] module.
        ),
        workspace_mounts: runtime_surfaces.workspace_mount_policy_for_test().clone(),
        memory_mounts: runtime_surfaces.memory_mounts_for_test().clone(),
        system_extensions_lifecycle_mounts: runtime_surfaces
            .system_extensions_lifecycle_mounts_for_test()
            .clone(),
        extension_surface_source: ExtensionCapabilitySurfaceSource::default(),
        input_resolver,
        result_writer,
        milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
        skill_activation_source: None,
        project_service: Arc::clone(&runtime_surfaces.project_service),
        trajectory_observer: None,
        outbound_preferences_service: None,
        outbound_preference_write_requires_approval: false,
        approval_settings: Arc::new(ironclaw_approvals::EmptyApprovalSettingsProvider),
        approval_requests: runtime_surfaces.approval_requests_for_test().clone(),
        capability_leases: runtime_surfaces.capability_leases_for_test().clone(),
        gate_record_store: Arc::new(ironclaw_approvals::GateRecordStore::new(
            crate::wrap_scoped(Arc::new(ironclaw_filesystem::InMemoryBackend::new())),
        )),
        replay_payload_store: Arc::new(ironclaw_capabilities::ReplayPayloadStore::new(
            crate::wrap_scoped(Arc::new(ironclaw_filesystem::InMemoryBackend::new())),
        )),
        external_tool_catalog: Arc::new(ironclaw_turns::InMemoryExternalToolCatalog::new()),
        unavailable_capability_ids: std::collections::HashSet::new(),
    };

    let port = factory
        .create_capability_port(&run_context)
        .await
        .expect("capability port"); // safety: test-only assertion in #[cfg(test)] module.
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible surface"); // safety: test-only assertion in #[cfg(test)] module.
    let input_ref = capability_io
        .register_provider_tool_call_input(
            &run_context,
            &provider_tool_call(provider_tool_name, arguments),
        )
        .await
        .expect("input ref"); // safety: test-only assertion in #[cfg(test)] module.

    let resolution = port
        .invoke_capability(LoopRequest {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: surface.version,
            capability_id: CapabilityId::new(capability_id).expect("capability id"), // safety: test-only literal id.
            input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("capability invocation"); // safety: test-only assertion in #[cfg(test)] module.

    let output = match &resolution {
        Resolution::Done(outcome) => outcome.refs.origin.as_ref().map(|result_ref| {
            capability_io
                .result_output(result_ref.as_str())
                .expect("result store read") // safety: test-only assertion in #[cfg(test)] module.
                .expect("completed tool result is stored") // safety: test-only assertion in #[cfg(test)] module.
        }),
        _ => None,
    };
    ToolOutcome { resolution, output }
}

struct ToolOutcome {
    resolution: Resolution,
    output: Option<serde_json::Value>,
}

fn assert_tool_succeeded(outcome: &ToolOutcome, label: &str) {
    let Resolution::Done(done) = &outcome.resolution else {
        panic!("{label} should complete, got {:?}", outcome.resolution);
    };
    assert!(
        matches!(
            done.verdict,
            ironclaw_host_api::resolution::ToolVerdict::Success
        ),
        "{label} should succeed, got {:?}",
        done.verdict
    );
}

/// Assert the tool completed with a recoverable failure whose model-visible
/// diagnostic contains `needle`. The coding engines error with the
/// pinned `not found` text when the caller's workspace root does not exist
/// yet — the v1 tools returned empty results for the same input.
fn assert_tool_failed_containing(outcome: &ToolOutcome, label: &str, needle: &str) {
    let Resolution::Done(done) = &outcome.resolution else {
        panic!("{label} should complete, got {:?}", outcome.resolution);
    };
    let ironclaw_host_api::resolution::ToolVerdict::RecoverableFailure { diagnostic, .. } =
        &done.verdict
    else {
        panic!("{label} should fail recoverably, got {:?}", done.verdict);
    };
    let text = diagnostic.model_visible_text().unwrap_or_else(|| {
        panic!("{label} failure must carry free-text cause, got {diagnostic:?}")
    });
    assert!(
        text.contains(needle),
        "{label} failure diagnostic must contain {needle:?}, got {text:?}"
    );
}

async fn read_composed_path(services: &RebornRuntimeStores, path: &str) -> Option<String> {
    let filesystem = services
        .local_runtime_for_test()
        .expect("local runtime substrate") // safety: test-only assertion in #[cfg(test)] module.
        .extension_filesystem_for_test();
    match filesystem
        .read_file(&VirtualPath::new(path).expect("virtual path")) // safety: test-only literal path.
        .await
    {
        Ok(bytes) => Some(String::from_utf8(bytes).expect("utf-8 body")), // safety: test writes utf-8.
        Err(ironclaw_filesystem::FilesystemError::NotFound { .. }) => None,
        Err(error) => panic!("composed filesystem read failed for {path}: {error}"),
    }
}

#[tokio::test]
async fn hosted_profile_lands_agent_workspace_writes_in_the_callers_own_subtree() {
    let storage = tempfile::tempdir().expect("temp storage root"); // safety: test-only assertion in #[cfg(test)] module.
    let services = crate::factory::build_runtime_substrate(
        crate::deployment::local_filesystem_build_input_with_profile(
            RebornCompositionProfile::HostedSingleTenantVolume,
            "hosted-owner",
            storage.path().to_path_buf(),
        )
        // The volume preview profile's default runtime policy denies the
        // filesystem lane outright, which would mask the assertion under test.
        // Pin the local-host policy so the only variable is the deployment's
        // workspace scoping decision.
        .with_runtime_policy(
            crate::standalone_runtime_policy().expect("local-host policy resolves"),
        ),
    )
    .await
    .expect("hosted services build"); // safety: test-only assertion in #[cfg(test)] module.

    write_workspace_file_as(&services, "alice", "note.txt", "alice-body").await;
    write_workspace_file_as(&services, "bob", "note.txt", "bob-body").await;

    assert_eq!(
        read_composed_path(
            &services,
            &format!("/projects/workspace/tenants/{TENANT}/users/alice/note.txt"),
        )
        .await
        .as_deref(),
        Some("alice-body"),
        "alice's write must land in alice's own subtree"
    );
    assert_eq!(
        read_composed_path(
            &services,
            &format!("/projects/workspace/tenants/{TENANT}/users/bob/note.txt"),
        )
        .await
        .as_deref(),
        Some("bob-body"),
        "bob writing the same relative path must not overwrite alice's file"
    );
    assert_eq!(
        read_composed_path(&services, "/projects/workspace/note.txt").await,
        None,
        "no hosted write may land in the shared workspace root the WebUI browser \
         no longer reads"
    );
}

#[tokio::test]
async fn standalone_profile_keeps_agent_workspace_writes_at_the_shared_root() {
    let storage = tempfile::tempdir().expect("temp storage root"); // safety: test-only assertion in #[cfg(test)] module.
    let services = crate::factory::build_runtime_substrate(
        crate::deployment::local_filesystem_build_input_with_profile(
            RebornCompositionProfile::Standalone,
            "standalone-owner",
            storage.path().to_path_buf(),
        ),
    )
    .await
    .expect("standalone services build"); // safety: test-only assertion in #[cfg(test)] module.

    write_workspace_file_as(&services, "solo", "note.txt", "solo-body").await;

    assert_eq!(
        read_composed_path(&services, "/projects/workspace/note.txt")
            .await
            .as_deref(),
        Some("solo-body"),
        "a single-user deployment keeps the shared workspace root its host \
         aliases and browser address"
    );
    assert_eq!(
        read_composed_path(
            &services,
            &format!("/projects/workspace/tenants/{TENANT}/users/solo/note.txt"),
        )
        .await,
        None,
        "standalone must not silently relocate writes into a per-user subtree"
    );
}

async fn run_context_for(label: &str, owner: &UserId) -> LoopRunContext {
    let resolved = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .expect("profile resolves"); // safety: test-only assertion in #[cfg(test)] module.
    LoopRunContext::new(
        TurnScope::new_with_owner(
            TenantId::new(TENANT).expect("tenant id"), // safety: test-only literal id.
            Some(AgentId::new(format!("agent-{label}")).expect("agent id")), // safety: test-only literal id.
            Some(ProjectId::new(format!("project-{label}")).expect("project id")), // safety: test-only literal id.
            ThreadId::new(format!("thread-{label}")).expect("thread id"), // safety: test-only literal id.
            Some(owner.clone()),
        ),
        TurnId::new(),
        TurnRunId::new(),
        resolved,
    )
}

async fn enable_global_auto_approve(
    services: &RebornRuntimeStores,
    run_context: &LoopRunContext,
    user_id: &UserId,
) {
    let runtime_surfaces = services
        .local_runtime_for_test()
        .expect("local runtime substrate"); // safety: test-only assertion in #[cfg(test)] module.
    let mut scope = run_context.scope.to_resource_scope();
    scope.user_id = user_id.clone();
    ironclaw_approvals::AutoApproveSettingStorePort::set(
        runtime_surfaces.auto_approve_settings_for_test().as_ref(),
        ironclaw_approvals::AutoApproveSettingInput {
            updated_by: ironclaw_host_api::scope::Principal::User(user_id.clone()),
            scope,
            enabled: true,
        },
    )
    .await
    .expect("enabling global auto-approve should succeed"); // safety: test-only assertion in #[cfg(test)] module.
}

fn provider_tool_call(name: &str, arguments: serde_json::Value) -> ProviderToolCall {
    ProviderToolCall {
        provider_id: "test-provider".to_string(),
        provider_model_id: "test-model".to_string(),
        turn_id: Some("provider-turn-1".to_string()),
        id: "call-1".to_string(),
        name: ProviderToolName::new(name).expect("provider tool name"), // safety: test-only literal.
        arguments,
        response_reasoning: None,
        reasoning: None,
        signature: None,
    }
}

/// A brand-new caller has no workspace subtree on disk yet. The coding engines
/// fail with the pinned `not found` text until some path creates the
/// directory; the caller's FIRST write must succeed, create missing parents,
/// and make the workspace visible to the very next read/glob.
#[tokio::test]
async fn fresh_caller_reads_an_empty_workspace_then_writes_into_it() {
    let storage = tempfile::tempdir().expect("temp storage root"); // safety: test-only assertion in #[cfg(test)] module.
    let services = crate::factory::build_runtime_substrate(
        crate::deployment::local_filesystem_build_input_with_profile(
            RebornCompositionProfile::HostedSingleTenantVolume,
            "hosted-owner",
            storage.path().to_path_buf(),
        )
        .with_runtime_policy(
            crate::standalone_runtime_policy().expect("local-host policy resolves"),
        ),
    )
    .await
    .expect("hosted services build"); // safety: test-only assertion in #[cfg(test)] module.

    // Nothing has ever written for `newcomer`, so their
    // `tenants/{tenant}/users/newcomer` directory does not exist.
    let read_ws = invoke_workspace_tool_as(
        &services,
        "newcomer",
        "builtin_read",
        CODING_READ_CAPABILITY_ID,
        serde_json::json!({ "path": "/workspace" }),
    )
    .await;
    assert_tool_failed_containing(&read_ws, "read on a fresh caller's workspace", "not found");

    let globbed = invoke_workspace_tool_as(
        &services,
        "newcomer",
        "builtin_glob",
        ironclaw_host_runtime::GLOB_CAPABILITY_ID,
        serde_json::json!({ "path": "**/*" }),
    )
    .await;
    assert_tool_failed_containing(
        &globbed,
        "glob on a fresh caller's workspace",
        "Path not found",
    );

    let grepped = invoke_workspace_tool_as(
        &services,
        "newcomer",
        "builtin_grep",
        ironclaw_host_runtime::GREP_CAPABILITY_ID,
        serde_json::json!({ "pattern": "anything" }),
    )
    .await;
    assert_tool_failed_containing(
        &grepped,
        "grep on a fresh caller's workspace",
        "Path not found",
    );

    // The first write from that same fresh caller must succeed and become
    // visible to the very next read, including into a nested path whose parent
    // directories do not exist yet.
    write_workspace_file_as(&services, "newcomer", "first.txt", "newcomer-body").await;
    write_workspace_file_as(
        &services,
        "newcomer",
        "notes/deep/second.txt",
        "nested-body",
    )
    .await;

    assert_eq!(
        read_composed_path(
            &services,
            &format!("/projects/workspace/tenants/{TENANT}/users/newcomer/notes/deep/second.txt"),
        )
        .await
        .as_deref(),
        Some("nested-body"),
        "a fresh caller's first write must create missing parent directories"
    );

    let relisted = invoke_workspace_tool_as(
        &services,
        "newcomer",
        "builtin_glob",
        ironclaw_host_runtime::GLOB_CAPABILITY_ID,
        serde_json::json!({ "path": "**/*" }),
    )
    .await;
    assert_tool_succeeded(&relisted, "glob after the first write");
    let relisted_output = relisted.output.expect("glob returns output");
    assert!(
        relisted_output["output"]
            .as_str()
            .is_some_and(|output| output.contains("first.txt")),
        "the freshly written file must be listable, got {relisted_output}"
    );

    assert_eq!(
        read_composed_path(
            &services,
            &format!("/projects/workspace/tenants/{TENANT}/users/newcomer/first.txt"),
        )
        .await
        .as_deref(),
        Some("newcomer-body"),
        "the write still lands in the caller's own subtree"
    );
}
