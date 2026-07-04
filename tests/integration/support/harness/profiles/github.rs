//! github domain tools profiles (populated by the profile migration).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ironclaw_host_api::{
    CapabilityId, CredentialStageError, MountPermissions, RuntimeKind, SecretHandle, UserId,
};
use ironclaw_host_runtime::{READ_FILE_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID};
use ironclaw_network::NetworkHttpEgress;
use ironclaw_reborn_composition::ProductLiveCapabilityIo;

use super::super::super::github as github_support;
use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{
    HarnessResult, HostRuntimeCapabilityHarness, RecordingNetworkHttpEgress,
    RecordingRuntimeHttpEgress, bundled_extension_provider_trust, local_dev_all_effects,
    local_dev_host_runtime_with_registry_and_egress, wildcard_test_policy, workspace_mounts,
};

/// C-JOURNEY convergence seam: surfaces the file-tool approval-gate
/// capabilities (`write_file`/`read_file`, `PermissionMode::Ask` — same
/// grant shape as `file_tools_requiring_approval`) AND a single GitHub
/// capability (`github.get_repo`) on the SAME `build_reborn_services`
/// local-dev runtime — the one wired with the
/// `run_state`/`approval_requests`/`capability_leases` stores BOTH gate
/// classes' resume paths need (`new_with_options` -> `build_reborn_services`).
///
/// Distinct from `github_issue_tools_auth_required` (a separate,
/// lower-level `HostRuntimeServices` build with a hardcoded
/// `FixedRuntimeCredentialAccountResolver` and no run_state store — see
/// that constructor's doc comment): this harness's `github.*` credential
/// resolves through the REAL `ProductAuthRuntimeCredentialResolver`
/// (`factory.rs`, wired unconditionally by `build_reborn_services`). No
/// GitHub credential account is seeded at construction (unlike
/// `extension_lifecycle_tools`, which seeds all four bundled providers via
/// `.with_seed_extension_credentials()`).
///
/// **Gate chaining (empirically verified, not assumed):** the global
/// auto-approve toggle this harness disables (for the file-tool arm) is
/// NOT capability-scoped, so `github.get_repo` first raises a real
/// `TurnStatus::BlockedApproval` too. Approving re-dispatches the
/// still-uncredentialed capability, which blocks AGAIN at a real
/// `TurnStatus::BlockedAuth` (`CredentialStageError::AuthRequired`).
/// `RebornIntegrationHarness::resolve_auth_gate` seeds the account
/// (`seed_github_credential_account`) and resumes, letting the SAME
/// parked capability re-dispatch and complete — the happy-path auth
/// resume the `github_issue_tools_auth_required` fixture cannot do. See
/// `scenario_auth_then_approval_journey`'s module doc for the full
/// approval->auth chain a caller must drive.
///
/// **Making `github.*` genuinely dispatchable (not just granted) needed
/// two additive test-support seams, both required together:**
/// 1. `capability_ids`/`additional_provider_trust` alone are NOT enough —
///    they only populate the harness-authority grant layer. The runtime's
///    OWN dispatchable registry (`build_local_runtime`'s
///    `local_dev_builtin_extension_registry()`) contains only first-party
///    builtins + the four lifecycle capabilities; bundled packages
///    (github, gmail, …) live in a SEPARATE `AvailableExtensionCatalog`
///    used for search only. Without registry presence, a scripted
///    `github.*` call silently never reaches `invoke_capability` (the run
///    completes with zero recorded invocations). Fixed via
///    `RebornServices::publish_bundled_extension_for_test`
///    (`factory.rs`, new `#[cfg(feature = "test-support")]` accessor) —
///    reaches the SAME `ActiveExtensionPublisher::publish` step
///    `builtin.extension_activate` calls, called directly at harness
///    construction instead of via a scripted install/activate handshake.
/// 2. Registry presence alone still isn't sufficient: `build_local_runtime`
///    mounts `/system/extensions` at an EMPTY per-harness tempdir, so the
///    runtime fails to compile `wasm/github_tool.wasm` at dispatch time
///    (`Failed{host_creation_failed}`) even once the package metadata is
///    registered. Fixed by copying the REAL asset directory
///    (`github_support::asset_root()`, already used by the
///    `github_issue_tools_*` harnesses) into this harness's own tempdir
///    mount (`copy_dir_recursive`) — no new fixtures, reuses the existing
///    on-disk asset tree.
///
/// Runtime policy is left at `None` (like `file_tools_requiring_approval`,
/// NOT the `LocalDevYolo` policy `extension_lifecycle_tools` uses) so the
/// file tools' real `PermissionMode::Ask` gate is preserved; the two seams
/// above are independent of the runtime-policy profile.
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
        user_id: UserId::new("reborn-e2e-file-github-auth-user")?,
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
        ..ToolsProfile::new("reborn-e2e-file-github-auth-tools")?
    })
}

/// C-JOURNEY convergence seam: surfaces the file-tool approval-gate
/// capabilities (`write_file`/`read_file`, `PermissionMode::Ask` — same
/// grant shape as `file_tools_requiring_approval`) AND a single GitHub
/// capability (`github.get_repo`) on the SAME `build_reborn_services`
/// local-dev runtime — the one wired with the
/// `run_state`/`approval_requests`/`capability_leases` stores BOTH gate
/// classes' resume paths need (`new_with_options` -> `build_reborn_services`).
///
/// Distinct from `github_issue_tools_auth_required` (a separate,
/// lower-level `HostRuntimeServices` build with a hardcoded
/// `FixedRuntimeCredentialAccountResolver` and no run_state store — see
/// that constructor's doc comment): this harness's `github.*` credential
/// resolves through the REAL `ProductAuthRuntimeCredentialResolver`
/// (`factory.rs`, wired unconditionally by `build_reborn_services`). No
/// GitHub credential account is seeded at construction (unlike
/// `extension_lifecycle_tools`, which seeds all four bundled providers via
/// `.with_seed_extension_credentials()`).
///
/// **Gate chaining (empirically verified, not assumed):** the global
/// auto-approve toggle this harness disables (for the file-tool arm) is
/// NOT capability-scoped, so `github.get_repo` first raises a real
/// `TurnStatus::BlockedApproval` too. Approving re-dispatches the
/// still-uncredentialed capability, which blocks AGAIN at a real
/// `TurnStatus::BlockedAuth` (`CredentialStageError::AuthRequired`).
/// `RebornIntegrationHarness::resolve_auth_gate` seeds the account
/// (`seed_github_credential_account`) and resumes, letting the SAME
/// parked capability re-dispatch and complete — the happy-path auth
/// resume the `github_issue_tools_auth_required` fixture cannot do. See
/// `scenario_auth_then_approval_journey`'s module doc for the full
/// approval->auth chain a caller must drive.
///
/// **Making `github.*` genuinely dispatchable (not just granted) needed
/// two additive test-support seams, both required together:**
/// 1. `capability_ids`/`additional_provider_trust` alone are NOT enough —
///    they only populate the harness-authority grant layer. The runtime's
///    OWN dispatchable registry (`build_local_runtime`'s
///    `local_dev_builtin_extension_registry()`) contains only first-party
///    builtins + the four lifecycle capabilities; bundled packages
///    (github, gmail, …) live in a SEPARATE `AvailableExtensionCatalog`
///    used for search only. Without registry presence, a scripted
///    `github.*` call silently never reaches `invoke_capability` (the run
///    completes with zero recorded invocations). Fixed via
///    `RebornServices::publish_bundled_extension_for_test`
///    (`factory.rs`, new `#[cfg(feature = "test-support")]` accessor) —
///    reaches the SAME `ActiveExtensionPublisher::publish` step
///    `builtin.extension_activate` calls, called directly at harness
///    construction instead of via a scripted install/activate handshake.
/// 2. Registry presence alone still isn't sufficient: `build_local_runtime`
///    mounts `/system/extensions` at an EMPTY per-harness tempdir, so the
///    runtime fails to compile `wasm/github_tool.wasm` at dispatch time
///    (`Failed{host_creation_failed}`) even once the package metadata is
///    registered. Fixed by copying the REAL asset directory
///    (`github_support::asset_root()`, already used by the
///    `github_issue_tools_*` harnesses) into this harness's own tempdir
///    mount (`copy_dir_recursive`) — no new fixtures, reuses the existing
///    on-disk asset tree.
///
/// Runtime policy is left at `None` (like `file_tools_requiring_approval`,
/// NOT the `LocalDevYolo` policy `extension_lifecycle_tools` uses) so the
/// file tools' real `PermissionMode::Ask` gate is preserved; the two seams
/// above are independent of the runtime-policy profile.
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
/// **Credential injection runs through two mechanisms here, not one — worth
/// knowing before you change either.** The authorizer's
/// `InjectCredentialAccountOnce` obligation is one path. The
/// `local_dev_host_runtime_with_registry_and_egress` helper this calls into
/// separately auto-wires `SharedHostWasmRuntimeCredentials` with product-auth
/// restaging via `try_with_wasm_runtime` (since both `.with_secret_store` and
/// `.with_runtime_credential_account_resolver` are always set on that path),
/// which independently resolves the GitHub manifest's declared
/// `runtime_credentials` and stages the same secret. That staging path runs
/// unconditionally on every WASM HTTP call (`WasmRuntimeHttpAdapter::request`)
/// — it is not gated on the authorizer's `Decision`. So a test asserting on the
/// injected header proves the *end-to-end* wire outcome, not that the
/// authorizer's obligation specifically is the sole producer of the header.
///
/// As currently wired (manually verified once, not re-checked by CI — treat as
/// current-harness observation, not a guaranteed contract): removing the
/// obligation does not make the call fall back to an unauthenticated request;
/// the run instead hangs and never reaches `Completed`. That's why the
/// mutation-verify in `reborn_integration_secret_injection.rs` proves the
/// obligation's secret reaches the wire by flipping the secret *value* (a fast,
/// specific assertion failure) rather than by removing the obligation (which
/// would only yield a slow, ambiguous timeout — a poor mutation-test signal).
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
    Ok(HostRuntimeCapabilityHarness {
        runtime,
        approval_parts: None,
        auto_approve_settings: None,
        pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
        io: Arc::new(ProductLiveCapabilityIo::default()),
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
        process_port: None,
        profile_filesystem: None,
        project_service: None,
        skill_activation_source: None,
        attachment_test_support: None,
        outbound_target_tools: None,
        scope_capability_by_run_owner: false,
        product_auth: None,
        tool_permission_overrides: None,
    })
}
