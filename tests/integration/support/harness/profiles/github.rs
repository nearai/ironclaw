//! GitHub domain tools profiles.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ironclaw_host_api::{
    CapabilityId, CredentialStageError, MountPermissions, RuntimeKind, SecretHandle, UserId,
};
use ironclaw_host_runtime::{READ_FILE_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID};
use ironclaw_network::NetworkHttpEgress;

use super::super::super::github as github_support;
use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{
    HarnessResult, HostRuntimeCapabilityHarness, RecordingNetworkHttpEgress,
    RecordingRuntimeHttpEgress, bundled_extension_provider_trust, local_dev_all_effects,
    local_dev_host_runtime_with_registry_and_egress, wildcard_test_policy, workspace_mounts,
};

/// C-JOURNEY convergence seam: surfaces the file-tool approval-gate
/// capabilities (`write_file`/`read_file` @ `Ask`) AND a single GitHub
/// capability (`github.get_repo`) on the SAME `build_reborn_services`
/// local-dev runtime (the one wired with the stores both gate classes'
/// resume paths need). Distinct from `github_issue_tools_auth_required`
/// (a separate, lower-level build with a hardcoded credential resolver):
/// here `github.*` resolves through the REAL
/// `ProductAuthRuntimeCredentialResolver`, and no credential account is
/// seeded at construction.
///
/// **Gate chaining (empirically verified):** the disabled global
/// auto-approve is NOT capability-scoped, so `github.get_repo` first raises
/// `BlockedApproval`; approving re-dispatches the still-uncredentialed
/// capability, which blocks AGAIN at `BlockedAuth`.
/// `RebornIntegrationHarness::resolve_auth_gate` seeds the account and
/// resumes, letting the SAME parked capability complete — see
/// `scenario_auth_then_approval_journey`'s module doc for the full chain.
///
/// Making `github.*` genuinely dispatchable (not just granted) needs two
/// seams: (1) `RebornServices::publish_bundled_extension_for_test` registers
/// it in the runtime's OWN dispatchable registry (capability_ids/
/// additional_provider_trust alone only populate the harness-authority grant
/// layer and would silently no-op); (2) `copy_dir_recursive` copies the real
/// github asset directory into this harness's tempdir mount, since
/// `build_local_runtime` otherwise mounts `/system/extensions` empty and WASM
/// compilation fails at dispatch time.
///
/// Runtime policy is left `None` (not `LocalDevYolo`) so file tools' real
/// `PermissionMode::Ask` gate is preserved.
pub(crate) fn file_and_github_auth_tools_profile() -> HarnessResult<ToolsProfile> {
    // Hermetic guard: `new_with_options`'s `build_local_runtime` defaults to
    // a REAL `ReqwestNetworkTransport` when no test egress is supplied
    // (`factory.rs`). This harness surfaces a `github.*` WASM capability
    // that crosses HTTP, so it MUST override the network egress or the
    // post-resume dispatch would attempt a live network call.
    let github_fixture_response =
        br#"{"id":1,"full_name":"octocat/hello-world","private":false}"#.to_vec();
    let network_egress: Arc<dyn NetworkHttpEgress> = Arc::new(
        RecordingNetworkHttpEgress::with_body(github_fixture_response),
    );
    Ok(ToolsProfile {
        capability_ids: vec![
            CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?,
            CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
            CapabilityId::new("github.get_repo")?,
        ],
        effect_kinds: local_dev_all_effects(),
        options: HostRuntimeHarnessOptions::new(
            workspace_mounts(MountPermissions::read_write_list_delete())?,
            None,
        )
        .with_network_http_egress_for_test(network_egress)
        .with_activated_bundled_extension(github_support::extension_package()?),
        network_policy_override: Some(wildcard_test_policy()),
        provider_trust_override: Some(bundled_extension_provider_trust()?),
        post_construct_asset_copy: Some((
            github_support::asset_root(),
            std::path::PathBuf::from("local-dev/system/extensions/github"),
        )),
        auto_approve_default: Some(false),
        ..ToolsProfile::new(
            "reborn-e2e-file-github-auth-tools",
            "reborn-e2e-file-github-auth-user",
        )?
    })
}

/// See [`file_and_github_auth_tools_profile`].
pub(crate) async fn file_and_github_auth_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    file_and_github_auth_tools_profile()?.build().await
}

/// Wires the GitHub first-party WASM capabilities behind `GithubHarnessAuthorizer`.
/// See `github_issue_tools_with_credential_result` for the credential-injection
/// coupling this relies on (T0-SECRET-INJECT).
pub(crate) async fn github_issue_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    // Credential account resolves to a real handle → capability dispatches.
    github_issue_tools_with_credential_result(Ok(SecretHandle::new("github_manual_access")?))
}

/// E-AUTHGATE: the GitHub extension wired so its credential account resolver
/// returns `AuthRequired`, raising a `TurnStatus::BlockedAuth` gate when a
/// `github.*` capability is dispatched. Used by `RebornIntegrationGroup::live_auth_gate`.
pub(crate) async fn github_issue_tools_auth_required() -> HarnessResult<HostRuntimeCapabilityHarness>
{
    github_issue_tools_with_credential_result(Err(CredentialStageError::AuthRequired))
}

/// Shared GitHub-extension constructor (E-AUTHGATE): the only difference
/// between the happy-path and auth-blocked variants is the credential account
/// resolver result, so the full `Self {..}` literal lives here once.
///
/// Credential injection runs through two mechanisms, not one: the
/// authorizer's `InjectCredentialAccountOnce` obligation, AND
/// `local_dev_host_runtime_with_registry_and_egress`'s independent
/// `SharedHostWasmRuntimeCredentials` restaging (runs unconditionally on
/// every WASM HTTP call, not gated on the authorizer's `Decision`). So a test
/// asserting the injected header proves the end-to-end wire outcome, not
/// that the obligation alone produced it.
///
/// Empirically verified: removing the obligation does not fall back to an
/// unauthenticated request — the run hangs and never reaches `Completed`.
/// That's why `reborn_integration_secret_injection.rs`'s mutation-verify
/// flips the secret *value* (a fast, specific failure) rather than removing
/// the obligation (a slow, ambiguous timeout).
fn github_issue_tools_with_credential_result(
    credential_account_result: Result<SecretHandle, CredentialStageError>,
) -> HarnessResult<HostRuntimeCapabilityHarness> {
    let root = Arc::new(tempfile::tempdir()?);
    let storage_root = root.path().join("local-dev");
    let workspace_root = storage_root.join("workspace");
    std::fs::create_dir_all(&workspace_root)?;
    let github_fixture_response =
        br#"{"object":{"sha":"abc123def4567890abc123def4567890abc123de"},"ok":true}"#.to_vec();
    let runtime_http_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        github_fixture_response.clone(),
    ));
    let network_egress = Arc::new(RecordingNetworkHttpEgress::with_body(
        github_fixture_response,
    ));
    let runtime = local_dev_host_runtime_with_registry_and_egress(
        storage_root.clone(),
        github_support::extension_registry()?,
        runtime_http_egress.clone(),
        network_egress.clone(),
        credential_account_result,
    )?;
    let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
    let (io, result_writer_io) = super::super::default_capability_io_pair();
    Ok(HostRuntimeCapabilityHarness {
        runtime,
        approval_parts: None,
        auto_approve_settings: None,
        pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
        io: Mutex::new(io),
        result_writer_io: Mutex::new(result_writer_io),
        durable_capability_io_thread_service: Mutex::new(None),
        durable_capability_io_requested: false,
        root,
        workspace_root,
        mounts,
        capability_mount_overrides: Vec::new(),
        capability_ids: github_support::capability_ids()?,
        runtime_kind: RuntimeKind::Wasm,
        effect_kinds: github_support::effect_kinds(),
        network_policy: github_support::api_policy(),
        secrets: github_support::secret_handles()?,
        provider_id: github_support::provider_id()?,
        additional_provider_trust: Vec::new(),
        user_id: UserId::new("reborn-e2e-github-user")?,
        invocations: Arc::new(Mutex::new(Vec::new())),
        results: Arc::new(Mutex::new(Vec::new())),
        http_egress: Some(runtime_http_egress),
        network_egress: Some(network_egress),
        real_egress_transport: None,
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
        reborn_services: None,
    })
}
