fn capability_provider_contracts() -> ironclaw_extensions::HostApiContractRegistry {
    let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(
            ironclaw_extensions::CapabilityProviderHostApiContract::new()
                .expect("capability provider contract"),
        ))
        .expect("register capability provider contract");
    contracts
}
use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_auth::{GOOGLE_CALENDAR_EVENTS_SCOPE, GOOGLE_CALENDAR_READONLY_SCOPE};

#[derive(Default)]
struct SlackDmOpenNetworkEgress {
    calls: AtomicUsize,
}

#[async_trait]
impl ironclaw_network::NetworkHttpEgress for SlackDmOpenNetworkEgress {
    async fn execute(
        &self,
        request: ironclaw_network::NetworkHttpRequest,
    ) -> Result<ironclaw_network::NetworkHttpResponse, ironclaw_network::NetworkHttpError> {
        assert!(
            request.url.ends_with("/api/conversations.open"),
            "unexpected Slack request: {}",
            request.url
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        let body = br#"{"ok":true,"channel":{"id":"D-RUNTIME-RACE"}}"#.to_vec();
        Ok(ironclaw_network::NetworkHttpResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            usage: ironclaw_network::NetworkUsage {
                request_bytes: request.body.len() as u64,
                response_bytes: body.len() as u64,
                resolved_ip: None,
            },
            body,
        })
    }
}

#[test]
fn persistent_grantee_resolver_maps_outbound_delivery_target_set_to_synthetic_provider() {
    let registry = Arc::new(ironclaw_extensions::ExtensionRegistry::new());
    let resolver =
        super::RegistryPersistentApprovalGranteeResolver::new(registry).expect("resolver builds");
    let capability_id =
        CapabilityId::new(crate::outbound::OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID)
            .expect("capability id");
    let expected_provider =
        crate::outbound::outbound_delivery_synthetic_provider().expect("synthetic provider id");

    assert_eq!(
        ironclaw_product::PersistentApprovalGranteeResolver::persistent_approval_grantee(
            &resolver,
            &capability_id
        ),
        Some(Principal::Extension(expected_provider))
    );
}

#[test]
fn persistent_grantee_resolver_maps_registered_capability_to_provider() {
    let manifest = r#"
schema_version = "reborn.extension_manifest.v2"
id = "approval-provider"
name = "approval-provider"
version = "0.1.0"
description = "approval provider"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/approval-provider.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "approval-provider.write"
description = "write"
effects = ["external_write"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/write.input.json"
output_schema_ref = "schemas/write.output.json"
"#;
    let manifest = ironclaw_extensions::ExtensionManifest::parse(
        manifest,
        ironclaw_extensions::ManifestSource::HostBundled,
        &ironclaw_host_api::HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .expect("manifest parses");
    let package = ironclaw_extensions::ExtensionPackage::from_manifest(
        manifest,
        ironclaw_host_api::VirtualPath::new("/system/extensions/approval-provider").expect("root"),
    )
    .expect("package builds");
    let mut registry = ironclaw_extensions::ExtensionRegistry::new();
    registry.insert(package).expect("package inserts");
    let resolver = super::RegistryPersistentApprovalGranteeResolver::new(Arc::new(registry))
        .expect("resolver builds");
    let capability_id = CapabilityId::new("approval-provider.write").expect("capability id");
    let expected_provider =
        ironclaw_host_api::ExtensionId::new("approval-provider").expect("extension id");

    assert_eq!(
        ironclaw_product::PersistentApprovalGranteeResolver::persistent_approval_grantee(
            &resolver,
            &capability_id,
        ),
        Some(Principal::Extension(expected_provider))
    );
}

#[tokio::test]
async fn runtime_channel_identity_bind_uses_deployment_channel_before_user_activation() {
    let root = tempfile::tempdir().expect("tempdir");
    let network_egress = Arc::new(SlackDmOpenNetworkEgress::default());
    let build_input = crate::deployment::local_dev_build_input(
        "runtime-channel-bind-race-owner",
        root.path().join("local-dev"),
    )
    .with_runtime_policy(local_dev_runtime_policy())
    .with_network_http_egress_for_test(network_egress.clone())
    .with_channel_extension_bindings(vec![crate::input::ChannelExtensionBinding {
        extension_id: "slack".to_string(),
        adapter: Arc::new(ironclaw_slack_extension::SlackChannelAdapter),
        preference_target_codec: None,
    }]);
    let input =
        RebornRuntimeInput::from_build_input(build_input).with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-channel-bind-race-tenant".to_string(),
            agent_id: "runtime-channel-bind-race-agent".to_string(),
            source_binding_id: "runtime-channel-bind-race-source".to_string(),
            reply_target_binding_id: "runtime-channel-bind-race-reply".to_string(),
        });
    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let extension_management = &runtime.extension_management;
    // #6520 removed the port-side operator accessor: the tenant operator is
    // the owner the runtime was constructed with.
    let operator = UserId::new("runtime-channel-bind-race-owner").expect("valid lifecycle caller");
    let slack_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack")
        .expect("valid Slack ref");
    extension_management
        .install(slack_ref.clone(), &operator)
        .await
        .expect("install Slack before OAuth callback");

    runtime
        .admin_configuration_resolver
        .configure_admin_group_for_test(
            "extension.slack",
            vec![
                ("slack_bot_token".to_string(), "xoxb-test".to_string()),
                (
                    "slack_signing_secret".to_string(),
                    "signing-test".to_string(),
                ),
                ("slack_team_id".to_string(), "T-RUNTIME".to_string()),
                ("slack_api_app_id".to_string(), "A-RUNTIME".to_string()),
                ("slack_installation_id".to_string(), "I-RUNTIME".to_string()),
                ("slack_bot_user_id".to_string(), "U-BOT-RUNTIME".to_string()),
                (
                    "slack_oauth_client_id".to_string(),
                    "runtime-client-id".to_string(),
                ),
                (
                    "slack_oauth_client_secret".to_string(),
                    "runtime-client-secret".to_string(),
                ),
            ],
        )
        .await
        .expect("configure Slack channel deployment values");

    let binding_config = runtime
        .channel_identity_binding_config()
        .expect("production runtime channel identity binding config");
    let mut resource = ResourceScope::local_default(operator.clone(), InvocationId::new())
        .expect("callback resource scope");
    resource.tenant_id = runtime.thread_scope.tenant_id.clone();
    let callback_scope =
        ironclaw_auth::AuthProductScope::new(resource, ironclaw_auth::AuthSurface::Callback);
    let identity = ironclaw_auth::OAuthProviderIdentity::new(
        "U-RUNTIME",
        Some("T-RUNTIME".to_string()),
        None,
        Some("A-RUNTIME".to_string()),
    )
    .expect("proven Slack identity");
    let rollback = crate::extension_host::channel_identity::bind_channel_identities_for_callback(
        &binding_config,
        "slack",
        &callback_scope,
        Some(&identity),
    )
    .await
    .expect("bind Slack identity before activation")
    .expect("Slack callback maps to the installed channel extension");
    drop(rollback);

    let dm_targets = &runtime.channel_dm_target_store;

    let record = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(record) = dm_targets
                .load("slack", &operator)
                .await
                .expect("load deployment-owned DM target")
            {
                break record;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("deployment channel should provision before user activation");
    assert_eq!(record.target["conversation_id"], "D-RUNTIME-RACE");
    assert_eq!(network_egress.calls.load(Ordering::SeqCst), 1);

    extension_management
        .activate_with_prechecked_credentials_for_test(
            slack_ref,
            ExtensionActivationMode::Static,
            &operator,
        )
        .await
        .expect("activate Slack and publish the generic host snapshot");

    // Deployment registration wins over the compatibility activation snapshot,
    // so activation must not create a second delivery binding or provider call.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert_eq!(network_egress.calls.load(Ordering::SeqCst), 1);
}
/// Wiring guard: the `regex_skill_activation_enabled` flag from
/// [`RebornRuntimeInput`] must reach
/// [`SkillActivationSelectorConfig::regex_activation_enabled`]
/// unchanged, not get clobbered by a stray
/// `..SkillActivationSelectorConfig::default()` spread or by the
/// helper accidentally taking `Default::default()`. Covers the
/// composition-level path that
/// [`local_dev_filesystem_skill_context_source`] depends on.
#[test]
fn local_dev_selector_config_propagates_regex_activation_disabled() {
    let cfg = super::local_dev_selector_config(
        false,
        ironclaw_first_party_extension_ports::SkillInjectionMode::Listing,
    );
    assert!(
        !cfg.regex_activation_enabled,
        "regex_skill_activation_enabled=false must propagate into SkillActivationSelectorConfig"
    );
    // Local-dev uses criteria selection so a learned skill auto-activates on
    // a keyword/pattern match (the learn→reuse loop), not only on an
    // explicit `$name` mention. A revert to `ExplicitOnly` would silently
    // break auto-reuse, so lock it here.
    assert!(matches!(
        cfg.selection_mode,
        ironclaw_first_party_extension_ports::SkillActivationSelectionMode::ExplicitAndCriteria
    ));
}

#[test]
fn local_dev_selector_config_propagates_regex_activation_enabled() {
    let cfg = super::local_dev_selector_config(
        true,
        ironclaw_first_party_extension_ports::SkillInjectionMode::Listing,
    );
    assert!(
        cfg.regex_activation_enabled,
        "regex_skill_activation_enabled=true must propagate into SkillActivationSelectorConfig"
    );
}

#[test]
fn local_dev_selector_config_uses_large_skill_context_budget() {
    let cfg = super::local_dev_selector_config(
        true,
        ironclaw_first_party_extension_ports::SkillInjectionMode::Listing,
    );
    assert_eq!(
        cfg.max_context_tokens, 6000,
        "local-dev Reborn skill activation should match the legacy 6000-token skill budget"
    );
}

/// Wiring guard for the `IRONCLAW_REBORN_SKILL_INJECTION` env switch: the
/// parsed injection mode must reach
/// [`SkillActivationSelectorConfig::injection_mode`] unchanged (not get
/// clobbered by the `..default()` spread), and the parser must default to
/// `listing` while still accepting the `full` legacy escape hatch.
#[test]
fn local_dev_selector_config_propagates_injection_mode() {
    for mode in [
        ironclaw_first_party_extension_ports::SkillInjectionMode::Listing,
        ironclaw_first_party_extension_ports::SkillInjectionMode::Full,
    ] {
        let cfg = super::local_dev_selector_config(true, mode);
        assert_eq!(cfg.injection_mode, mode);
    }
}

#[test]
fn skill_injection_mode_parses_listing_full_and_defaults() {
    use ironclaw_first_party_extension_ports::SkillInjectionMode;
    for (value, expected) in [
        ("", SkillInjectionMode::Listing),
        ("listing", SkillInjectionMode::Listing),
        (" Listing ", SkillInjectionMode::Listing),
        ("full", SkillInjectionMode::Full),
        ("FULL", SkillInjectionMode::Full),
    ] {
        assert_eq!(
            super::skill_injection_mode_from(value).expect("valid mode parses"),
            expected,
            "value {value:?}"
        );
    }
    assert!(
        super::skill_injection_mode_from("bodies").is_err(),
        "unknown values must fail loud, not silently pick a mode"
    );
}

fn readiness_for_runtime_gate(
    profile: RebornCompositionProfile,
    state: RebornReadinessState,
    diagnostics: Vec<crate::RebornReadinessDiagnostic>,
) -> RebornReadiness {
    RebornReadiness {
        profile,
        state,
        facades: crate::RebornFacadeReadiness {
            host_runtime: true,
            turn_coordinator: true,
            product_auth: true,
        },
        workers: crate::RebornWorkerReadiness {
            turn_runner: true,
            trigger_poller: false,
        },
        diagnostics,
    }
}

/// Drive the cutover gate the way production does: build the deployment from
/// the profile, then gate on its `TrafficPolicy`.
fn cutover_gate(
    profile: RebornCompositionProfile,
    readiness: &crate::RebornReadiness,
) -> Result<(), RebornRuntimeError> {
    super::enforce_runtime_cutover_gate(
        &crate::deployment::DeploymentConfig::for_profile(profile, false),
        readiness,
    )
}

#[test]
fn runtime_cutover_gate_allows_validated_production_readiness() {
    let readiness = readiness_for_runtime_gate(
        RebornCompositionProfile::Production,
        RebornReadinessState::ProductionValidated,
        Vec::new(),
    );

    cutover_gate(RebornCompositionProfile::Production, &readiness)
        .expect("validated production runtime can start");
}

#[test]
fn runtime_cutover_gate_rejects_blocking_production_diagnostic() {
    let readiness = readiness_for_runtime_gate(
        RebornCompositionProfile::Production,
        RebornReadinessState::ProductionValidated,
        vec![
            crate::RebornReadinessDiagnostic::production_blocker(
                RebornCompositionProfile::Production,
                crate::RebornReadinessDiagnosticComponent::RuntimePolicy,
                crate::RebornReadinessDiagnosticReason::LocalOnly,
            )
            .expect("production profile should create a blocker"),
        ],
    );

    let error = cutover_gate(RebornCompositionProfile::Production, &readiness)
        .expect_err("blocking production diagnostic prevents runtime start");
    let RebornRuntimeError::InvalidArgument { reason } = error else {
        panic!("expected invalid argument, got {error:?}");
    };
    assert!(reason.contains("RuntimePolicy"), "reason: {reason}");
    assert!(reason.contains("LocalOnly"), "reason: {reason}");
}

#[test]
fn runtime_cutover_gate_rejects_migration_dry_run_runtime_start() {
    let readiness = readiness_for_runtime_gate(
        RebornCompositionProfile::MigrationDryRun,
        RebornReadinessState::MigrationDryRunValidated,
        Vec::new(),
    );

    let error = cutover_gate(RebornCompositionProfile::MigrationDryRun, &readiness)
        .expect_err("migration-dry-run cannot start live runtime");
    let RebornRuntimeError::InvalidArgument { reason } = error else {
        panic!("expected invalid argument, got {error:?}");
    };
    assert!(reason.contains("migration-dry-run"), "reason: {reason}");
}

#[test]
fn runtime_cutover_gate_allows_local_dev_readiness() {
    let readiness = readiness_for_runtime_gate(
        RebornCompositionProfile::LocalDev,
        RebornReadinessState::DevOnly,
        vec![crate::RebornReadinessDiagnostic::local_dev()],
    );

    cutover_gate(RebornCompositionProfile::LocalDev, &readiness)
        .expect("local-dev runtime is not production traffic");
}

#[test]
fn runtime_cutover_gate_allows_hosted_single_tenant_readiness() {
    let readiness = readiness_for_runtime_gate(
        RebornCompositionProfile::HostedSingleTenant,
        RebornReadinessState::HostedSingleTenantValidated,
        Vec::new(),
    );

    cutover_gate(RebornCompositionProfile::HostedSingleTenant, &readiness)
        .expect("validated hosted single-tenant runtime can start");
}

#[test]
fn runtime_cutover_gate_rejects_local_dev_readiness_for_hosted_single_tenant() {
    let readiness = readiness_for_runtime_gate(
        RebornCompositionProfile::HostedSingleTenant,
        RebornReadinessState::DevOnly,
        vec![crate::RebornReadinessDiagnostic::local_dev()],
    );

    let error = cutover_gate(RebornCompositionProfile::HostedSingleTenant, &readiness)
        .expect_err("hosted single-tenant runtime requires hosted readiness");
    let RebornRuntimeError::InvalidArgument { reason } = error else {
        panic!("expected invalid argument, got {error:?}");
    };
    assert!(reason.contains("hosted-single-tenant"), "reason: {reason}");
    assert!(
        reason.contains("HostedSingleTenantValidated"),
        "reason: {reason}"
    );
}

// ── scheduler wake wiring guard unit tests ───────────────────────────────
// These exercise `check_production_scheduler_wake_wiring` directly so the
// fail-closed negative branch is covered without needing a full libsql /
// postgres substrate.  The guard is gated on the same `libsql | postgres`
// cfg as the production composition path it protects.

#[test]
fn production_scheduler_wake_guard_rejects_production_with_absent_wiring() {
    let err =
        super::check_production_scheduler_wake_wiring(RebornCompositionProfile::Production, &None)
            .expect_err(
                "production runtime with absent scheduler wake wiring must be rejected fail-closed",
            );
    let RebornRuntimeError::InvalidArgument { reason } = err else {
        panic!("expected InvalidArgument, got {err:?}");
    };
    assert!(
        reason.contains("production runtime missing scheduler wake wiring"),
        "reason should name the missing wiring, got: {reason}"
    );
}

#[test]
fn production_scheduler_wake_guard_rejects_migration_dry_run_with_absent_wiring() {
    let err = super::check_production_scheduler_wake_wiring(
        RebornCompositionProfile::MigrationDryRun,
        &None,
    )
    .expect_err("migration-dry-run with absent scheduler wake wiring must be rejected fail-closed");
    let RebornRuntimeError::InvalidArgument { reason } = err else {
        panic!("expected InvalidArgument, got {err:?}");
    };
    assert!(
        reason.contains("production runtime missing scheduler wake wiring"),
        "reason should name the missing wiring, got: {reason}"
    );
}

#[test]
fn production_scheduler_wake_guard_passes_local_dev_with_absent_wiring() {
    // Local-dev never mints scheduler wake wiring; the guard must not
    // reject it (the scheduler loop mints its own channel on that path).
    super::check_production_scheduler_wake_wiring(RebornCompositionProfile::LocalDev, &None)
        .expect("local-dev is exempt from the scheduler wake wiring requirement");
}

use ironclaw_host_api::ProjectId;
use ironclaw_host_api::{
    ActivityId, AgentId, ApprovalRequestId, CapabilityId, InvocationId, Principal,
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
    Resolution, ResourceScope, TenantId, ThreadId, UserId,
    runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
        NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
    },
};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessage, HostManagedModelMessageRole, HostManagedModelRequest,
    HostManagedModelResponse, HostManagedToolResultContent, HostSkillContextBuildError,
    HostSkillContextCandidate, HostSkillContextSource, ModelCost, SpawnSubagentMode,
    SubagentKindId, SubagentThreadKind, SubagentThreadMetadata,
};
use ironclaw_product::{
    CREATE_THREAD_COMMAND, LifecyclePackageKind, LifecyclePackageRef, LifecycleProductPayload,
    LifecyclePublicState, LifecycleReadinessBlocker, ProductCreateThreadRequest,
    ProductListAutomationsRequest, ProductResolveGateRequest, ProductSetupExtensionRequest,
    ProductSubmitTurnRequest, ProductSurfaceCommandDescriptor, RESOLVE_GATE_COMMAND,
    RebornExtensionCredentialSetup, RebornOutboundPreferencesResponse,
    RebornSetupExtensionResponse, RebornSkillListResponse, RebornStreamEventsRequest,
    RebornStreamEventsResponse, RebornSubmitTurnResponse, RebornViewPage, RebornViewQuery,
    SUBMIT_TURN_COMMAND, approval_gate_ref,
};
use ironclaw_product::{ProductOutboundPayload, ProductProjectionItem};
use ironclaw_skills::SkillTrust;
use ironclaw_threads::{
    AppendToolResultReferenceRequest, EnsureThreadRequest, LoadContextMessagesRequest, MessageKind,
    MessageStatus, TOOL_RESULT_RECORD_READ_MAX_BYTES, ThreadHistoryRequest, ThreadScope,
    ToolResultSafeSummary,
};
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, GetRunStateRequest, IdempotencyKey,
    LoopResultRef, ReplyTargetBindingRef, SanitizedCancelReason, SourceBindingRef,
    SubmitChildRunRequest, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnId, TurnRunId,
    TurnScope, TurnStatus,
    run_profile::{
        InMemoryRunProfileResolver, LoopCapabilityPort, LoopRunContext, ModelProfileId,
        ProviderToolCall, RegisterProviderToolCallRequest, RunProfileResolutionRequest,
        RunProfileResolver, SkillVisibility, VisibleCapabilityRequest,
    },
};
use rust_decimal_macros::dec;

use crate::RebornRuntimeProcessBinding;
use crate::extension_host::extension_lifecycle::ExtensionActivationMode;
use crate::input::RebornHostBindings;
use crate::observability::hooks::HooksActivationConfig;
use crate::runtime_input::{
    PollSettings, RebornRuntimeIdentity, RebornRuntimeInput, TriggerFireAccessCheck,
    TriggerFireAccessChecker, TriggerFireAccessDecision, TriggerFireAccessError,
    TriggerPollerSettings,
};
use crate::{RebornCompositionProfile, RebornReadiness, RebornReadinessState, RebornRuntimeError};
use ironclaw_reborn_config::{RebornBootConfig, RebornHome, RebornProfile};

use super::{RebornSkillSourceKind, build_reborn_runtime};

const RUNTIME_POLL_TIMEOUT: Duration = Duration::from_secs(10);
const RUNTIME_SEND_TIMEOUT: Duration = Duration::from_secs(15);

async fn stop_turn_runner_worker_for_manual_state_test(runtime: &super::RebornRuntime) {
    runtime.turn_scheduler.stop_for_test().await;
}

/// The production-shaped credential gate for lifecycle projections: #6520's
/// `project` takes the caller's credential gate so credential-bearing
/// extensions can project `Active` (a `None` gate caps them at setup-needed).
fn runtime_extension_credential_gate(
    runtime: &super::RebornRuntime,
    user_id: &UserId,
) -> crate::extension_host::extension_activation_credentials::RuntimeExtensionActivationCredentialGate
{
    let mut scope = runtime.thread_scope.to_resource_scope();
    scope.user_id = user_id.clone();
    scope.mission_id = None;
    scope.thread_id = None;
    scope.invocation_id = InvocationId::new();
    crate::extension_host::extension_activation_credentials::RuntimeExtensionActivationCredentialGate::new(
        scope,
        runtime
            .product_auth
            .runtime_credential_account_selection_service(),
    )
}

/// Install an extension through the ProductSurface capability path the WebUI
/// uses. Install joins membership and auto-drives readiness (#6520 — there is
/// no separate Activate action); a credential-gated package parks on `auth`.
async fn install_webui_extension(
    api: &dyn ironclaw_host_api::ProductSurface,
    caller: ProductSurfaceCaller,
    package_ref: LifecyclePackageRef,
) {
    let response = api
        .invoke(
            caller,
            ironclaw_host_api::ProductSurfaceInvokeRequest {
                operation_id: CapabilityId::new(ironclaw_product::EXTENSION_INSTALL_CAPABILITY_ID)
                    .expect("extension install capability id"),
                input: serde_json::json!({ "extension_id": package_ref.id.as_str() }),
                activity_id: ActivityId::new(),
            },
        )
        .await
        .expect("install extension through ProductSurface capability");
    let resolution: Resolution =
        serde_json::from_value(response.output).expect("install resolution decodes");
    match resolution {
        Resolution::Done(outcome) if outcome.verdict.is_success() => {}
        Resolution::Blocked(blocked) if blocked.kind() == "auth" => {}
        other => panic!("extension install did not succeed: {other:?}"),
    }
}

fn local_dev_runtime_policy() -> EffectiveRuntimePolicy {
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

#[derive(Debug)]
struct RecordingGateway {
    reply: String,
    requests: Arc<StdMutex<Vec<HostManagedModelRequest>>>,
}

#[derive(Debug, Default)]
struct ModelOutageGateway {
    calls: AtomicUsize,
}

#[derive(Debug, Default)]
struct FailingSkillContextSource {
    calls: AtomicUsize,
}

#[derive(Debug, Default)]
struct ToolCallingGateway {
    calls: StdMutex<usize>,
    stream_model_calls: StdMutex<usize>,
    requests: StdMutex<Vec<HostManagedModelRequest>>,
}

#[derive(Debug, Default)]
struct AuthGateToolCallingGateway {
    requests: StdMutex<Vec<HostManagedModelRequest>>,
}

#[derive(Debug, Default)]
struct WorkspaceListingGateway {
    calls: StdMutex<usize>,
    requests: StdMutex<Vec<HostManagedModelRequest>>,
}

// Local-dev model replay is a bounded reference observation: for a
// result under the inline first-look preview cap (issue #5838,
// `LOCAL_DEV_RESULT_PREVIEW_MAX_BYTES`), the raw content legitimately
// appears inline in `detail.preview` so the model does not need a
// follow-up `result_read` call; only content beyond the cap requires one.
// Both fixtures below are well under the cap.
fn assert_local_dev_result_reference(tool_result: &HostManagedModelMessage, raw_marker: &str) {
    assert!(
        tool_result.content.contains(raw_marker),
        "a result under the first-look preview cap should appear inline in model replay: {}",
        tool_result.content
    );
    let Some(HostManagedToolResultContent::Reference { envelope }) =
        tool_result.tool_result_content.as_ref()
    else {
        panic!(
            "model replay should carry a result-reference envelope, got {:?}",
            tool_result.tool_result_content
        );
    };
    assert_eq!(envelope.version, 1);
    assert!(envelope.result_ref.starts_with("result:"));
    let observation = envelope
        .model_observation
        .as_ref()
        .expect("result-reference replay should include a model observation");
    assert_eq!(observation["schema_version"], serde_json::json!(1));
    assert_eq!(observation["status"], serde_json::json!("success"));
    assert_eq!(
        observation["detail"]["kind"],
        serde_json::json!("result_reference")
    );
    assert_eq!(
        observation["detail"]["result_ref"],
        serde_json::json!(envelope.result_ref)
    );
}

struct StaticSkillContextSource {
    candidates: Vec<HostSkillContextCandidate>,
}

#[derive(Debug)]
struct AllowingTriggerFireAccessChecker;

impl StaticSkillContextSource {
    fn new(candidates: Vec<HostSkillContextCandidate>) -> Self {
        Self { candidates }
    }
}

#[async_trait]
impl TriggerFireAccessChecker for AllowingTriggerFireAccessChecker {
    async fn check_trigger_fire_access(
        &self,
        _request: TriggerFireAccessCheck,
    ) -> Result<TriggerFireAccessDecision, TriggerFireAccessError> {
        Ok(TriggerFireAccessDecision::Allowed)
    }
}

#[async_trait]
impl HostSkillContextSource for StaticSkillContextSource {
    async fn load_skill_context_candidates(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<Vec<HostSkillContextCandidate>, HostSkillContextBuildError> {
        Ok(self.candidates.clone())
    }
}

#[async_trait]
impl HostManagedModelGateway for RecordingGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests
            .lock()
            .expect("recording gateway requests lock poisoned")
            .push(request);
        Ok(HostManagedModelResponse::assistant_reply(
            self.reply.clone(),
        ))
    }
}

#[async_trait]
impl HostManagedModelGateway for ModelOutageGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            "model service is unavailable",
        ))
    }
}

#[async_trait]
impl HostSkillContextSource for FailingSkillContextSource {
    async fn load_skill_context_candidates(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<Vec<HostSkillContextCandidate>, HostSkillContextBuildError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(HostSkillContextBuildError::SourceUnavailable)
    }
}

#[async_trait]
impl HostManagedModelGateway for ToolCallingGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        *self
            .stream_model_calls
            .lock()
            .expect("tool gateway stream count lock poisoned") += 1;
        self.requests
            .lock()
            .expect("tool gateway requests lock poisoned")
            .push(request);
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "expected capability-aware model path",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let call_index = {
            let mut calls = self.calls.lock().expect("tool gateway lock poisoned");
            let call_index = *calls;
            *calls += 1;
            call_index
        };
        self.requests
            .lock()
            .expect("tool gateway requests lock poisoned")
            .push(request.clone());
        if call_index == 1 {
            let tool_result = request
                .messages
                .iter()
                .find(|message| message.role == HostManagedModelMessageRole::ToolResult)
                .expect("second model call should include tool result");
            assert_local_dev_result_reference(tool_result, "hello from tool");
            let provider_call = tool_result
                .tool_result_provider_call
                .as_ref()
                .expect("provider replay metadata");
            assert_eq!(provider_call.provider_call_id, "call-1");
            assert_eq!(
                provider_call.capability_id,
                CapabilityId::new("builtin.echo").unwrap()
            );
            return Ok(HostManagedModelResponse::assistant_reply("tool ok"));
        }

        let surface = capabilities
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .map_err(model_capability_error)?;
        let echo_id = CapabilityId::new("builtin.echo").expect("echo id");
        assert!(
            surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.capability_id == echo_id),
            "builtin echo must be visible through local-dev runtime capability surface"
        );
        let echo_tool = capabilities
            .tool_definitions()
            .map_err(model_capability_error)?
            .into_iter()
            .find(|definition| definition.capability_id == echo_id)
            .expect("echo provider tool definition");
        let candidate = capabilities
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                turn_id: Some("provider-turn-1".to_string()),
                id: "call-1".to_string(),
                name: echo_tool.name,
                arguments: serde_json::json!({"message": "hello from tool"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }))
            .await
            .map_err(model_capability_error)?;
        Ok(HostManagedModelResponse::capability_calls(
            vec![candidate],
            "",
        ))
    }
}

/// A long echo argument, sized well over `TOOL_RESULT_RECORD_READ_MAX_BYTES`
/// (not just the old hardcoded 2KiB), so the default-observer test can
/// prove the payload is truncated before the observer sees it.
const LARGE_ECHO_MESSAGE: &str = "PAYLOAD0123456789ABCDEF_";
const LARGE_ECHO_TAIL: &str = "UNREPLAYED_RAW_TOOL_RESULT_TAIL";

fn large_echo_message() -> String {
    let repeat_count = TOOL_RESULT_RECORD_READ_MAX_BYTES / LARGE_ECHO_MESSAGE.len() + 1;
    format!(
        "Secretary of the Treasury: {}{}",
        LARGE_ECHO_MESSAGE.repeat(repeat_count),
        LARGE_ECHO_TAIL
    )
}

#[derive(Debug, Default)]
struct LargeEchoToolCallingGateway {
    calls: StdMutex<usize>,
}

#[async_trait]
impl HostManagedModelGateway for LargeEchoToolCallingGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "expected capability-aware model path",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let call_index = {
            let mut calls = self.calls.lock().expect("large echo gateway lock poisoned");
            let call_index = *calls;
            *calls += 1;
            call_index
        };
        if call_index == 1 {
            let tool_result = request
                .messages
                .iter()
                .find(|message| message.role == HostManagedModelMessageRole::ToolResult)
                .expect("second model call should include tool result");
            assert!(
                !tool_result.content.contains(LARGE_ECHO_TAIL),
                "raw tail must remain out of the model replay; got {} bytes",
                tool_result.content.len()
            );
            assert!(
                tool_result.content.contains("result_reference"),
                "model replay must carry a bounded result-reference observation"
            );
            assert!(
                tool_result.content.len() <= TOOL_RESULT_RECORD_READ_MAX_BYTES * 2,
                "tool result replay must stay within the envelope bound, got {} bytes",
                tool_result.content.len()
            );
            assert!(
                tool_result.content.contains("Secretary of the Treasury"),
                "the initial result-reference preview must retain ordinary document text"
            );
            let result_ref = match tool_result.tool_result_content.as_ref() {
                Some(HostManagedToolResultContent::Reference { envelope }) => {
                    envelope.result_ref.clone()
                }
                other => panic!("expected a result reference, got {other:?}"),
            };
            let result_read_id = CapabilityId::new("builtin.result_read").expect("reader id");
            let result_read_tool = capabilities
                .tool_definitions()
                .map_err(model_capability_error)?
                .into_iter()
                .find(|definition| definition.capability_id == result_read_id)
                .expect("result_read provider tool definition");
            let candidate = capabilities
                .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                    ProviderToolCall {
                        provider_id: "test-provider".to_string(),
                        provider_model_id: "test-model".to_string(),
                        turn_id: Some("provider-turn-2".to_string()),
                        id: "call-2".to_string(),
                        name: result_read_tool.name,
                        arguments: serde_json::json!({
                            "result_ref": result_ref,
                            "offset": 0,
                            "max_bytes": 2048,
                        }),
                        response_reasoning: None,
                        reasoning: None,
                        signature: None,
                    },
                ))
                .await
                .map_err(model_capability_error)?;
            return Ok(HostManagedModelResponse::capability_calls(
                vec![candidate],
                "",
            ));
        }
        if call_index == 2 {
            let tool_result = request
                .messages
                .iter()
                .rev()
                .find(|message| {
                    message.role == HostManagedModelMessageRole::ToolResult
                        && message
                            .tool_result_provider_call
                            .as_ref()
                            .is_some_and(|call| {
                                call.capability_id.as_str() == "builtin.result_read"
                            })
                })
                .expect("third model call should include result_read output");
            assert!(
                tool_result.content.contains(LARGE_ECHO_MESSAGE),
                "result_read must expose its bounded chunk to the model"
            );
            assert!(
                !tool_result.content.contains(LARGE_ECHO_TAIL),
                "the result_read response must remain bounded"
            );
            let observation: serde_json::Value =
                serde_json::from_str(&tool_result.content).expect("result_read observation");
            let detail = &observation["model_observation"]["detail"];
            assert_ne!(
                detail["result_ref"], observation["result_ref"],
                "result_read replay must retain the original result reference, not its own output ref"
            );
            assert!(
                detail["total_bytes"]
                    .as_u64()
                    .is_some_and(|total_bytes| total_bytes > 2048),
                "result_read replay must expose total bytes for continuation: {}",
                tool_result.content
            );
            assert_eq!(
                detail["next_offset"].as_u64(),
                Some(2048),
                "result_read replay must expose the next offset for continuation"
            );
            return Ok(HostManagedModelResponse::assistant_reply("tool ok"));
        }
        let echo_id = CapabilityId::new("builtin.echo").expect("echo id");
        let echo_tool = capabilities
            .tool_definitions()
            .map_err(model_capability_error)?
            .into_iter()
            .find(|definition| definition.capability_id == echo_id)
            .expect("echo provider tool definition");
        // Larger than both the observer preview and model replay preview.
        let big_message = large_echo_message();
        let candidate = capabilities
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                turn_id: Some("provider-turn-1".to_string()),
                id: "call-1".to_string(),
                name: echo_tool.name,
                arguments: serde_json::json!({ "message": big_message }),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }))
            .await
            .map_err(model_capability_error)?;
        Ok(HostManagedModelResponse::capability_calls(
            vec![candidate],
            "",
        ))
    }
}

#[async_trait]
impl HostManagedModelGateway for AuthGateToolCallingGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests
            .lock()
            .expect("auth-gate gateway requests lock poisoned")
            .push(request);
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "expected capability-aware model path",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests
            .lock()
            .expect("auth-gate gateway requests lock poisoned")
            .push(request);
        let notion_search_id = CapabilityId::new("notion.notion-search").expect("notion search id");
        let notion_tool = capabilities
            .tool_definitions()
            .map_err(model_capability_error)?
            .into_iter()
            .find(|definition| definition.capability_id == notion_search_id)
            .expect("activated Notion capability should be visible");
        let candidate = capabilities
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                turn_id: Some("provider-turn-auth-gate".to_string()),
                id: "call-auth-gate".to_string(),
                name: notion_tool.name,
                arguments: serde_json::json!({ "query": "project notes" }),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }))
            .await
            .map_err(model_capability_error)?;
        Ok(HostManagedModelResponse::capability_calls(
            vec![candidate],
            "",
        ))
    }
}

#[async_trait]
impl HostManagedModelGateway for WorkspaceListingGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests
            .lock()
            .expect("workspace gateway requests lock poisoned")
            .push(request);
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "expected capability-aware model path",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let call_index = {
            let mut calls = self.calls.lock().expect("workspace gateway lock poisoned");
            let call_index = *calls;
            *calls += 1;
            call_index
        };
        self.requests
            .lock()
            .expect("workspace gateway requests lock poisoned")
            .push(request.clone());
        if call_index > 0 {
            let tool_result = request
                .messages
                .iter()
                .find(|message| message.role == HostManagedModelMessageRole::ToolResult)
                .expect("second model call should include tool result");
            assert_local_dev_result_reference(tool_result, "workspace-sentinel.txt");
            return Ok(HostManagedModelResponse::assistant_reply("workspace ok"));
        }

        let list_dir_id = CapabilityId::new("builtin.list_dir").expect("list_dir id");
        let list_dir_tool = capabilities
            .tool_definitions()
            .map_err(model_capability_error)?
            .into_iter()
            .find(|definition| definition.capability_id == list_dir_id)
            .expect("list_dir provider tool definition");
        let candidate = capabilities
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                turn_id: Some("provider-turn-1".to_string()),
                id: "call-1".to_string(),
                name: list_dir_tool.name,
                arguments: serde_json::json!({"path": "/workspace"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }))
            .await
            .map_err(model_capability_error)?;
        Ok(HostManagedModelResponse::capability_calls(
            vec![candidate],
            "",
        ))
    }
}

fn model_capability_error(error: impl std::fmt::Display) -> HostManagedModelError {
    let safe_summary = error.to_string();
    HostManagedModelError::safe(HostManagedModelErrorKind::Unavailable, safe_summary)
}

static RUNTIME_ENV_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct RuntimeEnvGuardEntry {
    name: &'static str,
    effective: Option<String>,
    snapshot: ironclaw_common::env_helpers::RuntimeEnvSnapshot,
}

struct RuntimeEnvGuard {
    // Serializes tokio tests that mutate the runtime env overlay. The
    // set/remove helpers lock only the separate override map, not
    // ENV_MUTEX, so restoration can safely run while this guard is held.
    _async_lock: tokio::sync::MutexGuard<'static, ()>,
    _env_lock: std::sync::MutexGuard<'static, ()>,
    previous: Vec<RuntimeEnvGuardEntry>,
}

impl RuntimeEnvGuard {
    async fn set(name: &'static str, value: &str) -> Self {
        Self::with([(name, Some(value))]).await
    }

    async fn with<const N: usize>(vars: [(&'static str, Option<&str>); N]) -> Self {
        let async_lock = RUNTIME_ENV_TEST_LOCK.lock().await;
        let env_lock = ironclaw_common::env_helpers::lock_env();
        let previous = vars
            .iter()
            .map(|(name, _)| RuntimeEnvGuardEntry {
                name,
                effective: ironclaw_common::env_helpers::env_or_override(name),
                snapshot: ironclaw_common::env_helpers::snapshot_runtime_env(name),
            })
            .collect::<Vec<_>>();
        for (name, value) in vars {
            match value {
                Some(value) => ironclaw_common::env_helpers::set_runtime_env(name, value),
                None => ironclaw_common::env_helpers::mask_runtime_env(name),
            }
        }
        Self {
            _async_lock: async_lock,
            _env_lock: env_lock,
            previous,
        }
    }
}

impl Drop for RuntimeEnvGuard {
    fn drop(&mut self) {
        for previous in self.previous.iter().rev() {
            ironclaw_common::env_helpers::restore_runtime_env(previous.snapshot.clone());
            if !std::thread::panicking() {
                debug_assert_eq!(
                    ironclaw_common::env_helpers::env_or_override(previous.name),
                    previous.effective.clone(),
                    "RuntimeEnvGuard failed to restore {}",
                    previous.name
                );
            }
        }
    }
}

const NEARAI_AUTH_CAPTURE_MAX_REQUEST_BYTES: usize = 50 * 1024 * 1024;
const NEARAI_AUTH_CAPTURE_IO_TIMEOUT: Duration = Duration::from_secs(5);
const NEARAI_AUTH_CAPTURE_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

async fn write_nearai_auth_capture_bytes(
    stream: &mut tokio::net::TcpStream,
    response: &[u8],
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    match tokio::time::timeout(NEARAI_AUTH_CAPTURE_IO_TIMEOUT, stream.write_all(response)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(format!("write auth capture response failed: {error}")),
        Err(_) => Err(format!(
            "write auth capture response timed out after {:?}",
            NEARAI_AUTH_CAPTURE_IO_TIMEOUT
        )),
    }
}

async fn write_nearai_auth_capture_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    write_nearai_auth_capture_bytes(stream, response.as_bytes()).await
}

async fn start_nearai_auth_capture_server() -> (String, tokio::sync::oneshot::Receiver<String>) {
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpSocket;

    let socket = TcpSocket::new_v4().expect("test server socket");
    socket
        .bind("127.0.0.1:0".parse().expect("test server address"))
        .expect("test server binds");
    let listener = socket.listen(1024).expect("test server listens");
    let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
    let (auth_tx, auth_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        let mut auth_tx = Some(auth_tx);
        'connections: loop {
            let (mut stream, _) =
                match tokio::time::timeout(NEARAI_AUTH_CAPTURE_IDLE_TIMEOUT, listener.accept())
                    .await
                {
                    Ok(Ok(accepted)) => accepted,
                    Ok(Err(error)) => panic!("accept test request: {error}"),
                    Err(_) => break,
                };
            let mut buffer = Vec::new();
            let mut header_end = None;
            loop {
                let mut chunk = [0_u8; 1024];
                let read = match tokio::time::timeout(
                    NEARAI_AUTH_CAPTURE_IO_TIMEOUT,
                    stream.read(&mut chunk),
                )
                .await
                {
                    Ok(Ok(read)) => read,
                    Ok(Err(error)) => panic!("read test request: {error}"),
                    Err(_) => {
                        write_nearai_auth_capture_response(
                            &mut stream,
                            "408 Request Timeout",
                            "text/plain",
                            "request read timed out",
                        )
                        .await
                        .expect("write auth capture read timeout response");
                        continue 'connections;
                    }
                };
                if read == 0 {
                    break;
                }
                if buffer.len().saturating_add(read) > NEARAI_AUTH_CAPTURE_MAX_REQUEST_BYTES {
                    write_nearai_auth_capture_response(
                        &mut stream,
                        "413 Payload Too Large",
                        "text/plain",
                        "request too large",
                    )
                    .await
                    .expect("write auth capture oversized request response");
                    continue 'connections;
                }
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                    header_end = Some(index + 4);
                    break;
                }
            }

            let Some(header_end) = header_end else {
                write_nearai_auth_capture_response(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain",
                    "incomplete request headers",
                )
                .await
                .expect("write auth capture incomplete headers response");
                continue;
            };
            let headers = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
            let content_length = match headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            {
                Some((_, value)) => match value.trim().parse::<usize>() {
                    Ok(length) => length,
                    Err(_) => {
                        write_nearai_auth_capture_response(
                            &mut stream,
                            "400 Bad Request",
                            "text/plain",
                            "invalid content-length",
                        )
                        .await
                        .expect("write auth capture invalid content-length response");
                        continue;
                    }
                },
                None => {
                    write_nearai_auth_capture_response(
                        &mut stream,
                        "400 Bad Request",
                        "text/plain",
                        "missing content-length",
                    )
                    .await
                    .expect("write auth capture missing content-length response");
                    continue;
                }
            };
            let Some(request_len) = header_end.checked_add(content_length) else {
                write_nearai_auth_capture_response(
                    &mut stream,
                    "413 Payload Too Large",
                    "text/plain",
                    "request too large",
                )
                .await
                .expect("write auth capture overflow response");
                continue;
            };
            if request_len > NEARAI_AUTH_CAPTURE_MAX_REQUEST_BYTES {
                write_nearai_auth_capture_response(
                    &mut stream,
                    "413 Payload Too Large",
                    "text/plain",
                    "request too large",
                )
                .await
                .expect("write auth capture oversized content-length response");
                continue;
            }
            while buffer.len() < request_len {
                let mut chunk = [0_u8; 1024];
                let read = match tokio::time::timeout(
                    NEARAI_AUTH_CAPTURE_IO_TIMEOUT,
                    stream.read(&mut chunk),
                )
                .await
                {
                    Ok(Ok(read)) => read,
                    Ok(Err(error)) => panic!("read test body: {error}"),
                    Err(_) => {
                        write_nearai_auth_capture_response(
                            &mut stream,
                            "408 Request Timeout",
                            "text/plain",
                            "request body read timed out",
                        )
                        .await
                        .expect("write auth capture body timeout response");
                        continue 'connections;
                    }
                };
                if read == 0 {
                    write_nearai_auth_capture_response(
                        &mut stream,
                        "400 Bad Request",
                        "text/plain",
                        "incomplete request body",
                    )
                    .await
                    .expect("write auth capture incomplete body response");
                    continue 'connections;
                }
                let remaining = request_len - buffer.len();
                buffer.extend_from_slice(&chunk[..read.min(remaining)]);
            }

            let body = &buffer[header_end..request_len];
            let request_json = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<serde_json::Value>(body) {
                    Ok(value) => Some(value),
                    Err(_) => {
                        write_nearai_auth_capture_response(
                            &mut stream,
                            "400 Bad Request",
                            "text/plain",
                            "invalid json body",
                        )
                        .await
                        .expect("write auth capture invalid json response");
                        continue;
                    }
                }
            };
            let wants_stream = request_json
                .as_ref()
                .and_then(|value| value.get("stream"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let request_line = headers.lines().next().unwrap_or_default();
            let auth_header = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .map(|(_, value)| value.trim())
                .unwrap_or_default()
                .to_string();
            let is_chat_completion = request_line.contains("/v1/chat/completions");
            if is_chat_completion && wants_stream {
                let body = concat!(
                    r#"data: {"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
                    "\n\n",
                    "data: [DONE]\n\n"
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n{}",
                    body
                );
                write_nearai_auth_capture_bytes(&mut stream, response.as_bytes())
                    .await
                    .expect("write test streaming response");
            } else {
                let body = if is_chat_completion {
                    r#"{"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#
                } else {
                    r#"{"data":[]}"#
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                write_nearai_auth_capture_bytes(&mut stream, response.as_bytes())
                    .await
                    .expect("write test response");
            }

            if is_chat_completion {
                if let Some(auth_tx) = auth_tx.take() {
                    #[allow(clippy::let_underscore_must_use)]
                    // oneshot send; dropped receiver is expected
                    let _ = auth_tx.send(auth_header);
                }
                break;
            }
        }
    });

    (base_url, auth_rx)
}

async fn send_nearai_auth_capture_raw_request(base_url: &str, request: String) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let address = base_url
        .strip_prefix("http://")
        .expect("capture server URL has http prefix");
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect to capture server");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write raw capture request");
    stream.shutdown().await.expect("finish raw capture request");

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .await
        .expect("read raw capture response");
    response
}

#[tokio::test]
async fn nearai_auth_capture_server_rejects_incomplete_body() {
    let (base_url, _auth_rx) = start_nearai_auth_capture_server().await;
    let response = send_nearai_auth_capture_raw_request(
        &base_url,
        "POST /v1/chat/completions HTTP/1.1\r\nhost: localhost\r\ncontent-length: 32\r\n\r\n{\"stream\":true"
            .to_string(),
    )
    .await;

    assert!(
        response.starts_with("HTTP/1.1 400 Bad Request"),
        "expected incomplete body to be rejected, got: {response:?}"
    );
}

#[tokio::test]
async fn nearai_auth_capture_server_rejects_oversized_content_length() {
    let (base_url, _auth_rx) = start_nearai_auth_capture_server().await;
    let response = send_nearai_auth_capture_raw_request(
        &base_url,
        format!(
            "POST /v1/chat/completions HTTP/1.1\r\nhost: localhost\r\ncontent-length: {}\r\n\r\n",
            NEARAI_AUTH_CAPTURE_MAX_REQUEST_BYTES + 1
        ),
    )
    .await;

    assert!(
        response.starts_with("HTTP/1.1 413 Payload Too Large"),
        "expected oversized body to be rejected, got: {response:?}"
    );
}

#[tokio::test]
async fn nearai_auth_capture_server_rejects_missing_content_length() {
    let (base_url, _auth_rx) = start_nearai_auth_capture_server().await;
    let response = send_nearai_auth_capture_raw_request(
        &base_url,
        "POST /v1/chat/completions HTTP/1.1\r\nhost: localhost\r\n\r\n{}".to_string(),
    )
    .await;

    assert!(
        response.starts_with("HTTP/1.1 400 Bad Request"),
        "expected missing content-length to be rejected, got: {response:?}"
    );
    assert!(
        response.contains("missing content-length"),
        "expected missing content-length diagnostic, got: {response:?}"
    );
}

fn nearai_gateway_test_request() -> HostManagedModelRequest {
    HostManagedModelRequest {
        model_profile_id: ironclaw_turns::run_profile::ModelProfileId::new("interactive_model")
            .expect("model profile id"),
        messages: vec![ironclaw_loop_host::HostManagedModelMessage {
            role: HostManagedModelMessageRole::User,
            content: "hello model".to_string(),
            content_ref: ironclaw_turns::LoopMessageRef::new(
                "msg:22222222-2222-2222-2222-222222222222",
            )
            .expect("message ref"),
            tool_result_provider_call: None,
            tool_result_content: None,
            image_parts: Vec::new(),
        }],
        surface_version: None,
        resolved_model_route: None,
        run_id: TurnRunId::new(),
        turn_id: TurnId::new(),
    }
}

#[derive(Debug)]
struct RecordingLlmProvider {
    active_model: StdMutex<String>,
    requests: StdMutex<Vec<Option<String>>>,
}

impl RecordingLlmProvider {
    fn new(active_model: &str) -> Self {
        Self {
            active_model: StdMutex::new(active_model.to_string()),
            requests: StdMutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl ironclaw_llm::LlmProvider for RecordingLlmProvider {
    fn model_name(&self) -> &str {
        "recording-provider"
    }

    fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
        (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
    }

    async fn complete(
        &self,
        request: ironclaw_llm::CompletionRequest,
    ) -> Result<ironclaw_llm::CompletionResponse, ironclaw_llm::LlmError> {
        self.requests
            .lock()
            .expect("recording provider request lock poisoned")
            .push(request.model);
        Ok(ironclaw_llm::CompletionResponse {
            content: "ok".to_string(),
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: ironclaw_llm::FinishReason::Stop,
            reasoning: None,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        request: ironclaw_llm::ToolCompletionRequest,
    ) -> Result<ironclaw_llm::ToolCompletionResponse, ironclaw_llm::LlmError> {
        self.requests
            .lock()
            .expect("recording provider request lock poisoned")
            .push(request.model);
        Ok(ironclaw_llm::ToolCompletionResponse {
            content: Some("ok".to_string()),
            tool_calls: Vec::new(),
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: ironclaw_llm::FinishReason::Stop,
            reasoning: None,
            reasoning_details: None,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    fn active_model_name(&self) -> String {
        self.active_model
            .lock()
            .expect("recording provider active-model lock poisoned")
            .clone()
    }

    fn set_model(&self, model: &str) -> Result<(), ironclaw_llm::LlmError> {
        *self
            .active_model
            .lock()
            .expect("recording provider active-model lock poisoned") = model.to_string();
        Ok(())
    }
}

#[tokio::test]
async fn swappable_gateway_uses_current_active_model_for_requests() {
    let provider = Arc::new(RecordingLlmProvider::new("boot-model"));
    let raw: Arc<dyn ironclaw_llm::LlmProvider> = provider.clone();
    let session =
        ironclaw_llm::create_session_manager(ironclaw_llm::SessionConfig::default()).await;
    let bundle = super::wrap_swappable_gateway(raw, session, None).expect("gateway bundle");

    bundle
        .gateway
        .stream_model(nearai_gateway_test_request())
        .await
        .expect("first request");
    bundle
        .reload
        .reload_handle
        .primary_provider()
        .set_model("reloaded-model")
        .expect("set active model");
    bundle
        .gateway
        .stream_model(nearai_gateway_test_request())
        .await
        .expect("second request");

    let requests = provider
        .requests
        .lock()
        .expect("recording provider request lock poisoned");
    assert_eq!(
        *requests,
        vec![
            Some("boot-model".to_string()),
            Some("reloaded-model".to_string())
        ],
        "production gateway must not keep sending the model selected at boot"
    );
}

fn skill_md(name: &str, description: &str, prompt: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\nactivation:\n  keywords: [\"{name}\"]\n---\n\n{prompt}"
    )
}

fn user_skill_dir(
    storage_root: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
    name: &str,
) -> std::path::PathBuf {
    storage_root
        .join("tenants")
        .join(tenant_id)
        .join("users")
        .join(user_id)
        .join("skills")
        .join(name)
}

fn skill_md_with_setup_marker(name: &str, description: &str, marker: &str, prompt: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\nactivation:\n  keywords: [\"{name}\"]\n  setup_marker: \"{marker}\"\n---\n\n{prompt}"
    )
}

fn recorded_request_count(requests: &StdMutex<Vec<HostManagedModelRequest>>) -> usize {
    requests
        .lock()
        .expect("recording gateway requests lock poisoned")
        .len()
}

#[tokio::test]
async fn root_llm_gateway_bootstraps_nearai_session_token_from_env() {
    let _token_guard = RuntimeEnvGuard::set("NEARAI_SESSION_TOKEN", "sess_reborn_env_token").await;
    let session_dir = tempfile::tempdir().expect("session tempdir");
    let (base_url, auth_rx) = start_nearai_auth_capture_server().await;

    let config = ironclaw_llm::LlmConfig {
        backend: "nearai".to_string(),
        session: ironclaw_llm::SessionConfig {
            auth_base_url: base_url.clone(),
            session_path: session_dir.path().join("session.json"),
        },
        nearai: ironclaw_llm::NearAiConfig {
            model: "test-model".to_string(),
            cheap_model: None,
            base_url,
            api_key: None,
            fallback_model: None,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
            smart_routing_cascade: false,
        },
        provider: None,
        bedrock: None,
        gemini_oauth: None,
        openai_codex: None,
        request_timeout_secs: 5,
        cheap_model: None,
        smart_routing_cascade: false,
        max_retries: 0,
        circuit_breaker_threshold: None,
        circuit_breaker_recovery_secs: 30,
        response_cache_enabled: false,
        response_cache_ttl_secs: 3600,
        response_cache_max_entries: 1000,
    };
    let session = ironclaw_llm::create_session_manager(config.session.clone()).await;
    let built = ironclaw_llm::build_static_provider_chain(&config, Arc::clone(&session))
        .await
        .expect("provider chain builds from config");
    let bundle = super::wrap_swappable_gateway(built, session, None).expect("gateway builds");
    let response = bundle
        .gateway
        .stream_model(nearai_gateway_test_request())
        .await
        .expect("gateway calls NEAR AI provider");

    assert_eq!(response.safe_text_deltas, vec!["ok".to_string()]);
    let auth_header = tokio::time::timeout(Duration::from_secs(2), auth_rx)
        .await
        .expect("chat request should be captured")
        .expect("auth header should be sent by capture server");
    assert_eq!(auth_header, "Bearer sess_reborn_env_token");
}

#[tokio::test]
async fn runtime_nearai_mcp_bootstraps_from_nearai_session_token() {
    let _token_guard = RuntimeEnvGuard::set("NEARAI_SESSION_TOKEN", "sess_reborn_mcp_token").await;
    let root = tempfile::tempdir().expect("tempdir");
    let session_dir = tempfile::tempdir().expect("session tempdir");
    let local_dev_root = root.path().join("local-dev");

    let config = ironclaw_llm::LlmConfig {
        backend: "nearai".to_string(),
        session: ironclaw_llm::SessionConfig {
            auth_base_url: "https://private.near.ai".to_string(),
            session_path: session_dir.path().join("session.json"),
        },
        nearai: ironclaw_llm::NearAiConfig {
            model: "test-model".to_string(),
            cheap_model: None,
            base_url: "https://private.near.ai".to_string(),
            api_key: None,
            fallback_model: None,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
            smart_routing_cascade: false,
        },
        provider: None,
        bedrock: None,
        gemini_oauth: None,
        openai_codex: None,
        request_timeout_secs: 5,
        cheap_model: None,
        smart_routing_cascade: false,
        max_retries: 0,
        circuit_breaker_threshold: None,
        circuit_breaker_recovery_secs: 30,
        response_cache_enabled: false,
        response_cache_ttl_secs: 3600,
        response_cache_max_entries: 1000,
    };
    let llm = crate::runtime_input::ResolvedRebornLlm::from_llm_config(config);

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-nearai-session-mcp-owner",
            local_dev_root,
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_resolved_llm(llm)
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-nearai-session-mcp-tenant".to_string(),
        agent_id: "runtime-nearai-session-mcp-agent".to_string(),
        source_binding_id: "runtime-nearai-session-mcp-source".to_string(),
        reply_target_binding_id: "runtime-nearai-session-mcp-reply".to_string(),
    });

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let extension_management = &runtime.extension_management;
    let nearai_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai").expect("valid ref");
    let operator = UserId::new("runtime-nearai-session-mcp-owner").expect("valid lifecycle caller");
    let credential_gate = runtime_extension_credential_gate(&runtime, &operator);
    let projection = extension_management
        .project(nearai_ref, &operator, Some(&credential_gate))
        .await
        .expect("NEAR AI MCP projected");
    assert_eq!(projection.phase, LifecyclePublicState::Active);

    let capabilities = extension_management
        .active_model_visible_capabilities()
        .await
        .expect("active capabilities");
    assert!(
        capabilities
            .iter()
            .any(|capability| capability.id.as_str() == "nearai.web_search"),
        "nearai.web_search should be active with NEAR AI session-token config"
    );
    stop_turn_runner_worker_for_manual_state_test(&runtime).await;
}

#[tokio::test]
async fn runtime_nearai_mcp_bootstraps_from_stored_nearai_api_key() {
    let _env_guard = RuntimeEnvGuard::with([
        ("NEARAI_SESSION_TOKEN", None),
        ("NEARAI_API_KEY", None),
        ("NEARAI_BASE_URL", None),
    ])
    .await;
    let root = tempfile::tempdir().expect("tempdir");
    let local_dev_root = root.path().join("local-dev");
    let session_dir = tempfile::tempdir().expect("session tempdir");

    let services = crate::factory::build_runtime_substrate(
        crate::deployment::local_dev_build_input(
            "runtime-nearai-stored-mcp-owner",
            local_dev_root.clone(),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .await
    .expect("services build for stored key seed");
    crate::LlmKeyStore::new(services.secret_store())
        .put(
            "nearai",
            ironclaw_secrets::SecretMaterial::from("sk-reborn-stored-nearai-mcp-key"),
        )
        .await
        .expect("stored key seeded");
    drop(services);

    let config = ironclaw_llm::LlmConfig {
        backend: "nearai".to_string(),
        session: ironclaw_llm::SessionConfig {
            auth_base_url: "https://private.near.ai".to_string(),
            session_path: session_dir.path().join("session.json"),
        },
        nearai: ironclaw_llm::NearAiConfig {
            model: "test-model".to_string(),
            cheap_model: None,
            base_url: "https://cloud-api.near.ai".to_string(),
            api_key: None,
            fallback_model: None,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
            smart_routing_cascade: false,
        },
        provider: None,
        bedrock: None,
        gemini_oauth: None,
        openai_codex: None,
        request_timeout_secs: 5,
        cheap_model: None,
        smart_routing_cascade: false,
        max_retries: 0,
        circuit_breaker_threshold: None,
        circuit_breaker_recovery_secs: 30,
        response_cache_enabled: false,
        response_cache_ttl_secs: 3600,
        response_cache_max_entries: 1000,
    };
    let llm = crate::runtime_input::ResolvedRebornLlm::from_llm_config(config);

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input("runtime-nearai-stored-mcp-owner", local_dev_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_resolved_llm(llm)
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-nearai-stored-mcp-tenant".to_string(),
        agent_id: "runtime-nearai-stored-mcp-agent".to_string(),
        source_binding_id: "runtime-nearai-stored-mcp-source".to_string(),
        reply_target_binding_id: "runtime-nearai-stored-mcp-reply".to_string(),
    });

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let extension_management = &runtime.extension_management;
    let nearai_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai").expect("valid ref");
    let operator = UserId::new("runtime-nearai-stored-mcp-owner").expect("valid lifecycle caller");
    let credential_gate = runtime_extension_credential_gate(&runtime, &operator);
    let projection = extension_management
        .project(nearai_ref, &operator, Some(&credential_gate))
        .await
        .expect("NEAR AI MCP projected");
    assert_eq!(projection.phase, LifecyclePublicState::Active);

    let capabilities = extension_management
        .active_model_visible_capabilities()
        .await
        .expect("active capabilities");
    assert!(
        capabilities
            .iter()
            .any(|capability| capability.id.as_str() == "nearai.web_search"),
        "nearai.web_search should be active with stored NEAR AI API key config"
    );
    stop_turn_runner_worker_for_manual_state_test(&runtime).await;
}

async fn nearai_mcp_runtime_access_secret(
    runtime: &super::RebornRuntime,
    owner_scope: ResourceScope,
) -> String {
    let product_auth = &runtime.product_auth;
    let auth_scope = ironclaw_auth::AuthProductScope::credential_owner(
        &owner_scope,
        ironclaw_auth::AuthSurface::Api,
    );
    let accounts = product_auth
        .credential_account_record_source()
        .accounts_for_owner(&auth_scope)
        .await
        .expect("NEAR AI product-auth accounts");
    let account = accounts
        .into_iter()
        .find(|account| {
            account.provider.as_str() == "nearai"
                && account.status == ironclaw_auth::CredentialAccountStatus::Configured
        })
        .expect("configured NEAR AI product-auth account");

    assert_eq!(account.scope.resource.tenant_id, owner_scope.tenant_id);
    assert_eq!(account.scope.resource.user_id, owner_scope.user_id);
    assert_eq!(account.scope.resource.agent_id, owner_scope.agent_id);
    assert_eq!(account.scope.resource.project_id, owner_scope.project_id);

    let handle = account.access_secret.expect("NEAR AI access secret");
    let store = runtime.secret_store();
    let lease = store
        .lease_once(&account.scope.resource, &handle)
        .await
        .expect("NEAR AI access secret lease");
    let material = store
        .consume(&account.scope.resource, lease.id)
        .await
        .expect("NEAR AI access secret material");
    secrecy::ExposeSecret::expose_secret(&material).to_string()
}

#[tokio::test]
async fn runtime_nearai_mcp_prebuild_api_key_is_not_replaced_by_stored_key() {
    let _env_guard = RuntimeEnvGuard::with([
        ("NEARAI_SESSION_TOKEN", None),
        ("NEARAI_API_KEY", None),
        ("NEARAI_BASE_URL", None),
    ])
    .await;
    let root = tempfile::tempdir().expect("tempdir");
    let local_dev_root = root.path().join("local-dev");
    let session_dir = tempfile::tempdir().expect("session tempdir");
    let owner = "runtime-nearai-prebuild-mcp-owner";
    let tenant = "runtime-nearai-prebuild-mcp-tenant";
    let agent = "runtime-nearai-prebuild-mcp-agent";

    let services = crate::factory::build_runtime_substrate(
        crate::deployment::local_dev_build_input(owner, local_dev_root.clone())
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .await
    .expect("services build for stored key seed");
    crate::LlmKeyStore::new(services.secret_store())
        .put(
            "nearai",
            ironclaw_secrets::SecretMaterial::from("sk-post-build-stored-nearai-mcp-key"),
        )
        .await
        .expect("stored key seeded");
    drop(services);

    let config = ironclaw_llm::LlmConfig {
        backend: "nearai".to_string(),
        session: ironclaw_llm::SessionConfig {
            auth_base_url: "https://private.near.ai".to_string(),
            session_path: session_dir.path().join("session.json"),
        },
        nearai: ironclaw_llm::NearAiConfig {
            model: "test-model".to_string(),
            cheap_model: None,
            base_url: "https://cloud-api.near.ai".to_string(),
            api_key: Some(secrecy::SecretString::from("sk-prebuild-nearai-mcp-key")),
            fallback_model: None,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
            smart_routing_cascade: false,
        },
        provider: None,
        bedrock: None,
        gemini_oauth: None,
        openai_codex: None,
        request_timeout_secs: 5,
        cheap_model: None,
        smart_routing_cascade: false,
        max_retries: 0,
        circuit_breaker_threshold: None,
        circuit_breaker_recovery_secs: 30,
        response_cache_enabled: false,
        response_cache_ttl_secs: 3600,
        response_cache_max_entries: 1000,
    };
    let llm = crate::runtime_input::ResolvedRebornLlm::from_llm_config(config);

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(owner, local_dev_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_resolved_llm(llm)
    .with_identity(RebornRuntimeIdentity {
        tenant_id: tenant.to_string(),
        agent_id: agent.to_string(),
        source_binding_id: "runtime-nearai-prebuild-mcp-source".to_string(),
        reply_target_binding_id: "runtime-nearai-prebuild-mcp-reply".to_string(),
    });

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let owner_scope = ResourceScope {
        tenant_id: TenantId::new(tenant).expect("tenant"),
        user_id: UserId::new(owner).expect("owner"),
        agent_id: Some(AgentId::new(agent).expect("agent")),
        project_id: None::<ProjectId>,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let material = nearai_mcp_runtime_access_secret(&runtime, owner_scope).await;

    assert_eq!(material, "sk-prebuild-nearai-mcp-key");
    stop_turn_runner_worker_for_manual_state_test(&runtime).await;
}

/// Counts how many times the runtime drives this provider and answers with a
/// fixed sentinel, so a test can prove an injected provider — not one built
/// from config — is the one the gateway actually calls.
struct CountingOverrideProvider {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl ironclaw_llm::LlmProvider for CountingOverrideProvider {
    fn model_name(&self) -> &str {
        "mock-override-model"
    }

    fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
        (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
    }

    async fn complete(
        &self,
        _request: ironclaw_llm::CompletionRequest,
    ) -> Result<ironclaw_llm::CompletionResponse, ironclaw_llm::LlmError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ironclaw_llm::CompletionResponse {
            content: "override-driven".to_string(),
            input_tokens: 0,
            output_tokens: 0,
            finish_reason: ironclaw_llm::FinishReason::Stop,
            reasoning: None,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ironclaw_llm::ToolCompletionRequest,
    ) -> Result<ironclaw_llm::ToolCompletionResponse, ironclaw_llm::LlmError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ironclaw_llm::ToolCompletionResponse {
            content: Some("override-driven".to_string()),
            tool_calls: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            finish_reason: ironclaw_llm::FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            reasoning: None,
            reasoning_details: None,
        })
    }
}

/// The LLM-provider-instrumentation seam: when a caller installs a factory
/// via `ResolvedRebornLlm::with_provider_factory` (how the bench wraps an
/// instrumented provider to capture reasoning / tokens / cost / system-prompt
/// / tool definitions), the gateway must drive the factory's output. Here the
/// factory ignores the config-built provider and returns a counting mock, so
/// if the factory were not applied the gateway would drive the config-built
/// provider (dead endpoint) instead of returning the mock's sentinel.
#[tokio::test]
async fn wrap_swappable_gateway_applies_provider_factory() {
    let session_dir = tempfile::tempdir().expect("session tempdir");
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mock: Arc<dyn ironclaw_llm::LlmProvider> = Arc::new(CountingOverrideProvider {
        calls: Arc::clone(&calls),
    });

    let config = ironclaw_llm::LlmConfig {
        backend: "nearai".to_string(),
        session: ironclaw_llm::SessionConfig {
            auth_base_url: "http://127.0.0.1:1".to_string(),
            session_path: session_dir.path().join("session.json"),
        },
        nearai: ironclaw_llm::NearAiConfig {
            model: "config-model-should-not-be-used".to_string(),
            cheap_model: None,
            base_url: "http://127.0.0.1:1".to_string(),
            api_key: None,
            fallback_model: None,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
            smart_routing_cascade: false,
        },
        provider: None,
        bedrock: None,
        gemini_oauth: None,
        openai_codex: None,
        request_timeout_secs: 5,
        cheap_model: None,
        smart_routing_cascade: false,
        max_retries: 0,
        circuit_breaker_threshold: None,
        circuit_breaker_recovery_secs: 30,
        response_cache_enabled: false,
        response_cache_ttl_secs: 3600,
        response_cache_max_entries: 1000,
    };

    let factory_mock = Arc::clone(&mock);
    let session = ironclaw_llm::create_session_manager(config.session.clone()).await;
    let built = ironclaw_llm::build_static_provider_chain(&config, Arc::clone(&session))
        .await
        .expect("provider chain builds from config");
    let bundle = super::wrap_swappable_gateway(
        built,
        session,
        Some(Arc::new(move |_built| Arc::clone(&factory_mock))),
    )
    .expect("gateway builds with the provider factory");

    let response = bundle
        .gateway
        .stream_model(nearai_gateway_test_request())
        .await
        .expect("gateway drives the factory-produced provider");

    assert_eq!(
        response.safe_text_deltas,
        vec!["override-driven".to_string()],
        "gateway must return the factory provider's response, not the config-built one"
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the override provider should be invoked exactly once"
    );
}

/// Provider wrapper that counts model calls and delegates to its inner — a
/// stand-in for the bench's instrumentation wrapper. Unlike
/// `CountingOverrideProvider`, it wraps `inner` so swapping the inner (via a
/// live reload of a `SwappableLlmProvider`) is observable through it.
struct CountingWrapperProvider {
    inner: Arc<dyn ironclaw_llm::LlmProvider>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl ironclaw_llm::LlmProvider for CountingWrapperProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(
        &self,
        request: ironclaw_llm::CompletionRequest,
    ) -> Result<ironclaw_llm::CompletionResponse, ironclaw_llm::LlmError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.complete(request).await
    }

    async fn complete_with_tools(
        &self,
        request: ironclaw_llm::ToolCompletionRequest,
    ) -> Result<ironclaw_llm::ToolCompletionResponse, ironclaw_llm::LlmError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.complete_with_tools(request).await
    }
}

/// Minimal nearai `LlmConfig` pointed at a dead endpoint: it *builds* lazily
/// (no connection at construction) but any model call errors. Enough to
/// exercise gateway/reload wiring without a network.
fn dead_endpoint_nearai_config(session_path: std::path::PathBuf) -> ironclaw_llm::LlmConfig {
    ironclaw_llm::LlmConfig {
        backend: "nearai".to_string(),
        session: ironclaw_llm::SessionConfig {
            auth_base_url: "http://127.0.0.1:1".to_string(),
            session_path,
        },
        nearai: ironclaw_llm::NearAiConfig {
            model: "config-model".to_string(),
            cheap_model: None,
            base_url: "http://127.0.0.1:1".to_string(),
            api_key: None,
            fallback_model: None,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
            smart_routing_cascade: false,
        },
        provider: None,
        bedrock: None,
        gemini_oauth: None,
        openai_codex: None,
        request_timeout_secs: 5,
        cheap_model: None,
        smart_routing_cascade: false,
        max_retries: 0,
        circuit_breaker_threshold: None,
        circuit_breaker_recovery_secs: 30,
        response_cache_enabled: false,
        response_cache_ttl_secs: 3600,
        response_cache_max_entries: 1000,
    }
}

/// Regression guard for Firat's review: the provider factory (caller
/// instrumentation) must survive a live config reload. `wrap_swappable_gateway`
/// wraps the factory over the `SwappableLlmProvider`, so reloading — which
/// swaps the swappable's *inner* — keeps the wrapper in the call path. If the
/// factory were applied to the bare provider instead, the first reload would
/// silently drop instrumentation and this test's post-reload count would stay
/// at 1.
#[tokio::test]
async fn provider_factory_survives_live_reload() {
    let session_dir = tempfile::tempdir().expect("session tempdir");
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_for_factory = Arc::clone(&calls);
    let factory: crate::runtime_input::RebornProviderFactory = Arc::new(move |inner| {
        Arc::new(CountingWrapperProvider {
            inner,
            calls: Arc::clone(&calls_for_factory),
        }) as Arc<dyn ironclaw_llm::LlmProvider>
    });

    let config = dead_endpoint_nearai_config(session_dir.path().join("session.json"));
    let session = ironclaw_llm::create_session_manager(config.session.clone()).await;
    let built = ironclaw_llm::build_static_provider_chain(&config, Arc::clone(&session))
        .await
        .expect("provider chain builds from config");
    let bundle = super::wrap_swappable_gateway(built, session, Some(factory))
        .expect("gateway builds with the provider factory");

    // First model call routes through the instrumentation wrapper. The dead
    // endpoint makes the underlying call error, but the wrapper counts before
    // delegating, so the result is irrelevant — only that it was observed.
    #[allow(clippy::let_underscore_must_use)]
    // dead endpoint errors by design; only the wrapper's observation count matters
    let _ = bundle
        .gateway
        .stream_model(nearai_gateway_test_request())
        .await;
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the instrumentation wrapper should observe the first model call"
    );

    // Live config reload: rebuild the chain and atomically swap the
    // swappable's inner provider — exactly what the WebUI settings path does.
    bundle
        .reload
        .reload_handle
        .reload(&config, Arc::clone(&bundle.reload.session))
        .await
        .expect("live reload rebuilds the provider chain");

    #[allow(clippy::let_underscore_must_use)]
    // dead endpoint errors by design; only the wrapper's observation count matters
    let _ = bundle
        .gateway
        .stream_model(nearai_gateway_test_request())
        .await;
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the instrumentation wrapper must still observe model calls after a live reload"
    );
}

/// Regression guard for the trace-recording gap: `IRONCLAW_RECORD_TRACE=1` on
/// the serve/run path must place a `RecordingLlm` in the turn provider chain.
/// The runtime builds turns through `wrap_swappable_gateway`, which never calls
/// `RecordingLlm::from_env`, and hot-reloads through
/// `build_provider_chain_components`, which also does not — so the recorder is
/// wired *only* via `ResolvedRebornLlm::with_env_trace_recording`. Nothing
/// pinned "serve + IRONCLAW_RECORD_TRACE ⇒ recorder attached" before, which is
/// exactly why the env "enabled" recording yet serve emitted nothing (the
/// committed reborn_qa fixtures were recorded through the in-process harness,
/// whose `build_provider_chain` path *does* wire the recorder).
///
/// This asserts the gate at the exact serve/run resolution seam. That the
/// attached factory actually wraps a recorder which records and flushes to disk
/// incrementally (no explicit `flush()`, matching serve's signalled shutdown)
/// is proven in
/// `ironclaw_llm::recording::tests::complete_flushes_incrementally_without_explicit_flush`
/// — the crate that owns `RecordingLlm` and can set real env vars, which this
/// `#![forbid(unsafe_code)]` crate cannot.
#[tokio::test]
async fn env_trace_recording_attaches_recorder_factory_only_when_enabled() {
    let session_dir = tempfile::tempdir().expect("session tempdir");
    let config = dead_endpoint_nearai_config(session_dir.path().join("session.json"));

    // Disabled: no factory attached; the resolved LLM is returned unchanged.
    {
        let _guard = RuntimeEnvGuard::with([("IRONCLAW_RECORD_TRACE", None)]).await;
        let disabled = crate::runtime_input::ResolvedRebornLlm::from_llm_config(config.clone())
            .with_env_trace_recording();
        assert!(
            disabled.provider_factory.is_none(),
            "no recording factory should attach when IRONCLAW_RECORD_TRACE is unset"
        );
    }

    // Enabled: the serve/run resolution path attaches the recording factory.
    {
        let _guard = RuntimeEnvGuard::set("IRONCLAW_RECORD_TRACE", "1").await;
        let enabled = crate::runtime_input::ResolvedRebornLlm::from_llm_config(config)
            .with_env_trace_recording();
        assert!(
            enabled.provider_factory.is_some(),
            "IRONCLAW_RECORD_TRACE must attach the recording provider factory on the \
             serve/run resolution path"
        );
    }
}

/// Regression guard for the benchmark instrumentation seam: a
/// `ResolvedRebornLlm` carrying a `provider_factory` must have that factory
/// invoked during `build_reborn_runtime`, i.e. the caller's instrumentation
/// wrapper is threaded into the cold-boot gateway.
///
/// PR #6174 collapsed the boot path to `build_placeholder_llm_gateway()`, which
/// hardcoded `None` for the factory, so `ResolvedRebornLlm::with_provider_factory`
/// silently never ran on the production path — the benchmark harness saw every
/// task fail with zero model calls (no instrumented provider). The
/// `provider_factory_survives_live_reload` test above exercises the
/// `wrap_swappable_gateway` helper directly with `Some(..)`, so it cannot catch
/// a boot path that never calls the helper with a factory at all. This drives
/// the real caller (`build_reborn_runtime`) instead.
#[tokio::test]
async fn provider_factory_runs_during_production_boot() {
    let root = tempfile::tempdir().expect("tempdir");
    let session_dir = tempfile::tempdir().expect("session tempdir");
    let local_dev_root = root.path().join("local-dev");

    let factory_ran = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let factory_ran_for_closure = Arc::clone(&factory_ran);
    // Identity decorator that only records that it was constructed: the factory
    // runs once, at gateway construction, to wrap the swappable provider.
    let factory: crate::runtime_input::RebornProviderFactory = Arc::new(move |inner| {
        factory_ran_for_closure.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        inner
    });

    let config = dead_endpoint_nearai_config(session_dir.path().join("session.json"));
    let llm = crate::runtime_input::ResolvedRebornLlm::from_llm_config(config)
        .with_provider_factory(factory);

    // No `boot` config is supplied, so the boot-time reload is skipped and the
    // dead endpoint is never contacted; the factory still wraps the swappable
    // at cold-boot construction.
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input("provider-factory-boot-owner", local_dev_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_resolved_llm(llm)
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "provider-factory-boot-tenant".to_string(),
        agent_id: "provider-factory-boot-agent".to_string(),
        source_binding_id: "provider-factory-boot-source".to_string(),
        reply_target_binding_id: "provider-factory-boot-reply".to_string(),
    });

    let _runtime = build_reborn_runtime(input).await.expect("runtime builds");

    assert_eq!(
        factory_ran.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the caller's provider_factory must be invoked once during boot so \
         instrumentation wraps the swappable gateway (regression: #6174 dropped it)"
    );
}

/// Regression pin for the journey-critical fix (PR #6174): a provider
/// selected purely through `config.toml` + a stored API key (no env var set)
/// must reach the turn-serving provider. This exercises the ONLY mechanism
/// that now applies a stored key to the live gateway — the post-construction
/// `RebornLlmReloadAdapter::reload()` invoked once inside
/// `build_reborn_runtime` — by supplying a real `boot` config (so the
/// reload adapter can re-resolve `[llm.default]` from disk) instead of
/// pre-baking the stored key into a directly-supplied `ResolvedRebornLlm`
/// (which no longer feeds the gateway at all).
#[tokio::test]
async fn local_dev_runtime_startup_uses_stored_nearai_api_key_after_restart() {
    // NOTE on isolation: this test does not need to override
    // `NEARAI_SESSION_PATH` / `NEARAI_AUTH_URL` (both env-only inputs to
    // `ironclaw_llm::resolution::nearai_session_config`, which the reload
    // adapter's config-file re-resolution invokes). `NearAiChatProvider::
    // resolve_bearer_token` checks `config.nearai.api_key` FIRST, before
    // ever touching the session manager — and `apply_stored_api_key` (called
    // by `RebornLlmReloadAdapter::reload`) sets exactly that field from the
    // seeded key below. So the session/auth-url defaults are constructed but
    // never read from disk or contacted over the network.
    let _env_guard = RuntimeEnvGuard::with([
        ("NEARAI_SESSION_TOKEN", None),
        ("NEARAI_API_KEY", None),
        ("NEARAI_BASE_URL", None),
    ])
    .await;
    let (base_url, auth_rx) = start_nearai_auth_capture_server().await;

    let root = tempfile::tempdir().expect("tempdir");
    let local_dev_root = root.path().join("local-dev");
    let config_home_dir = root.path().join("config-home");
    std::fs::create_dir_all(&config_home_dir).expect("config home dir");

    let services = crate::factory::build_runtime_substrate(
        crate::deployment::local_dev_build_input(
            "runtime-nearai-stored-key-owner",
            local_dev_root.clone(),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .await
    .expect("services build for stored key seed");
    crate::LlmKeyStore::new(services.secret_store())
        .put(
            "nearai",
            ironclaw_secrets::SecretMaterial::from("sk-reborn-stored-nearai-key"),
        )
        .await
        .expect("stored key seeded");
    drop(services);

    // Provider selection lives entirely in config.toml (mirrors an
    // onboard-style setup): no env var carries the key, only the
    // encrypted secret store does. `base_url` is overridden to the local
    // capture server so the live reload's re-built provider chain actually
    // calls it.
    std::fs::write(
        RebornHome::resolve_from_env_parts(
            Some(config_home_dir.as_os_str().to_os_string()),
            None,
            None,
        )
        .expect("valid reborn home")
        .config_file_path(),
        format!(
            "[llm.default]\nprovider_id = \"nearai\"\nmodel = \"test-model\"\nbase_url = \"{base_url}\"\n"
        ),
    )
    .expect("write config.toml");
    let boot = RebornBootConfig::new(
        RebornHome::resolve_from_env_parts(
            Some(config_home_dir.as_os_str().to_os_string()),
            None,
            None,
        )
        .expect("valid reborn home"),
        RebornProfile::LocalDev,
    );

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input("runtime-nearai-stored-key-owner", local_dev_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_boot_config(boot)
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-nearai-stored-key-tenant".to_string(),
        agent_id: "runtime-nearai-stored-key-agent".to_string(),
        source_binding_id: "runtime-nearai-stored-key-source".to_string(),
        reply_target_binding_id: "runtime-nearai-stored-key-reply".to_string(),
    });

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = runtime
        .send_user_message(&conversation, "hi")
        .await
        .expect("message sends");

    assert!(reply.is_successful_final_reply(), "reply: {reply:?}");
    let auth_header = tokio::time::timeout(Duration::from_secs(5), auth_rx)
        .await
        .expect("chat request should be captured")
        .expect("auth header should be sent by capture server");
    assert_eq!(auth_header, "Bearer sk-reborn-stored-nearai-key");

    runtime.shutdown().await.expect("runtime shutdown");
}

/// Runtime-store unification (branch `unify-runtime-store-graph`): every build
/// now composes the single unified runtime store graph, so the hook framework
/// is wired for a production libsql build exactly as it is for local-dev — the
/// old "hooks are not wired for production runtime launch" rejection premise no
/// longer holds (its `else if hooks_config.is_enabled()` branch in
/// `build_reborn_runtime` is now unreachable). This locks the new-but-correct
/// behavior: enabling hooks on a production runtime builds and validates
/// readiness instead of failing `MalformedConfig`.
#[tokio::test]
async fn production_runtime_wires_enabled_hooks_through_unified_runtime() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("reborn.db"))
            .build()
            .await
            .expect("libsql db"),
    );

    let input = RebornRuntimeInput::from_build_input(
        RebornHostBindings::libsql(
            crate::RebornCompositionProfile::Production,
            "runtime-production-hooks-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
        )
        .with_production_trust_policy(Arc::new(
            crate::builtin_first_party_trust_policy().expect("trust policy"),
        ))
        .with_runtime_policy(EffectiveRuntimePolicy {
            deployment: DeploymentMode::HostedMultiTenant,
            requested_profile: RuntimeProfile::SecureDefault,
            resolved_profile: RuntimeProfile::SecureDefault,
            filesystem_backend: FilesystemBackendKind::ScopedVirtual,
            process_backend: ProcessBackendKind::TenantSandbox,
            network_mode: NetworkMode::Deny,
            secret_mode: SecretMode::BrokeredHandles,
            approval_policy: ApprovalPolicy::AskAlways,
            audit_mode: AuditMode::Standard,
        })
        .with_runtime_process_binding(RebornRuntimeProcessBinding::tenant_sandbox(Arc::new(
            ironclaw_host_runtime::TenantSandboxProcessPort::new(Arc::new(
                RecordingSandboxTransport,
            )),
        ))),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-production-hooks-tenant".to_string(),
        agent_id: "runtime-production-hooks-agent".to_string(),
        source_binding_id: "runtime-production-hooks-source".to_string(),
        reply_target_binding_id: "runtime-production-hooks-reply".to_string(),
    })
    .with_hooks_config(HooksActivationConfig::enabled());

    let runtime = build_reborn_runtime(input)
        .await
        .expect("unified production runtime wires the hook framework when hooks are enabled");
    assert_eq!(
        runtime.readiness().state,
        RebornReadinessState::ProductionValidated
    );
    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn build_reborn_runtime_allows_validated_production_readiness() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("reborn.db"))
            .build()
            .await
            .expect("libsql db"),
    );
    let gateway = Arc::new(RecordingGateway {
        reply: "validated production runtime".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });

    let input = RebornRuntimeInput::from_build_input(
        RebornHostBindings::libsql(
            crate::RebornCompositionProfile::Production,
            "runtime-production-cutover-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
        )
        .with_production_trust_policy(Arc::new(
            crate::builtin_first_party_trust_policy().expect("trust policy"),
        ))
        .with_runtime_policy(EffectiveRuntimePolicy {
            deployment: DeploymentMode::HostedMultiTenant,
            requested_profile: RuntimeProfile::SecureDefault,
            resolved_profile: RuntimeProfile::SecureDefault,
            filesystem_backend: FilesystemBackendKind::ScopedVirtual,
            process_backend: ProcessBackendKind::TenantSandbox,
            network_mode: NetworkMode::Deny,
            secret_mode: SecretMode::BrokeredHandles,
            approval_policy: ApprovalPolicy::AskAlways,
            audit_mode: AuditMode::Standard,
        })
        .with_runtime_process_binding(RebornRuntimeProcessBinding::tenant_sandbox(Arc::new(
            ironclaw_host_runtime::TenantSandboxProcessPort::new(Arc::new(
                RecordingSandboxTransport,
            )),
        ))),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-production-cutover-tenant".to_string(),
        agent_id: "runtime-production-cutover-agent".to_string(),
        source_binding_id: "runtime-production-cutover-source".to_string(),
        reply_target_binding_id: "runtime-production-cutover-reply".to_string(),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input)
        .await
        .expect("validated production readiness should start runtime");

    assert_eq!(
        runtime.readiness().state,
        RebornReadinessState::ProductionValidated
    );
    assert!(runtime.readiness().diagnostics.is_empty());
    assert!(runtime.readiness().workers.turn_runner);

    runtime.shutdown().await.expect("runtime shutdown");
}

/// Runtime-store unification (branch `unify-runtime-store-graph`): the
/// trajectory observer is wired through the (now single, always-present)
/// capability path, so a production runtime observes turns exactly as local-dev
/// does. The old rejection guard (Firat's review) existed because a
/// non-local-dev runtime had no capability hook and would silently produce an
/// empty trajectory — that premise no longer holds (the `else` reject branch in
/// `build_reborn_runtime` is now unreachable), so supplying an observer is
/// accepted and wired rather than rejected.
#[tokio::test]
async fn build_reborn_runtime_wires_trajectory_observer_through_unified_runtime() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("reborn.db"))
            .build()
            .await
            .expect("libsql db"),
    );
    let gateway = Arc::new(ToolCallingGateway::default());
    let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
    let observer = Arc::new(RecordingTrajectoryObserver::default());

    let input = RebornRuntimeInput::from_build_input(
        RebornHostBindings::libsql(
            crate::RebornCompositionProfile::Production,
            "runtime-observer-reject-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
        )
        .with_production_trust_policy(Arc::new(
            crate::builtin_first_party_trust_policy().expect("trust policy"),
        ))
        .with_runtime_policy(EffectiveRuntimePolicy {
            deployment: DeploymentMode::HostedMultiTenant,
            requested_profile: RuntimeProfile::SecureDefault,
            resolved_profile: RuntimeProfile::SecureDefault,
            filesystem_backend: FilesystemBackendKind::ScopedVirtual,
            process_backend: ProcessBackendKind::TenantSandbox,
            network_mode: NetworkMode::Deny,
            secret_mode: SecretMode::BrokeredHandles,
            approval_policy: ApprovalPolicy::AskAlways,
            audit_mode: AuditMode::Standard,
        })
        .with_runtime_process_binding(RebornRuntimeProcessBinding::tenant_sandbox(Arc::new(
            ironclaw_host_runtime::TenantSandboxProcessPort::new(Arc::new(
                RecordingSandboxTransport,
            )),
        ))),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-observer-reject-tenant".to_string(),
        agent_id: "runtime-observer-reject-agent".to_string(),
        source_binding_id: "runtime-observer-reject-source".to_string(),
        reply_target_binding_id: "runtime-observer-reject-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_raw_trajectory_observer(observer.clone())
    .with_model_gateway_override(gateway_for_runtime);

    let runtime = build_reborn_runtime(input)
        .await
        .expect("unified production runtime accepts and wires a trajectory observer");
    assert_eq!(
        runtime.readiness().state,
        RebornReadinessState::ProductionValidated
    );
    let conversation = runtime.new_conversation().await.expect("conversation");
    runtime
        .enable_global_auto_approve_for_test(&conversation)
        .await;
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "use echo tool"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");
    assert_eq!(reply.status, TurnStatus::Completed, "reply: {reply:?}");
    assert_eq!(reply.text.as_deref(), Some("tool ok"));
    runtime.shutdown().await.expect("runtime shutdown");

    let inputs = observer.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1, "exactly one capability input observed");
    let (input_call_id, input_capability, arguments) = &inputs[0];
    assert!(!input_call_id.is_empty(), "input call_id should be present");
    assert_eq!(input_capability, "builtin.echo");
    assert_eq!(
        arguments,
        &serde_json::json!({"message": "hello from tool"}),
        "observer should receive the raw model-emitted tool arguments"
    );

    let results = observer.results.lock().expect("results lock");
    assert_eq!(results.len(), 1, "exactly one capability result observed");
    let (result_call_id, result_capability, output) = &results[0];
    assert_eq!(result_capability, "builtin.echo");
    assert_eq!(
        result_call_id, input_call_id,
        "result and input callbacks correlate by call_id"
    );
    assert!(
        output.to_string().contains("hello from tool"),
        "observer should receive the staged capability output, got {output}"
    );
}

#[derive(Debug)]
struct RecordingSandboxTransport;

#[async_trait]
impl ironclaw_host_runtime::SandboxCommandTransport for RecordingSandboxTransport {
    async fn run_command(
        &self,
        _request: ironclaw_host_runtime::CommandExecutionRequest,
    ) -> Result<
        ironclaw_host_runtime::CommandExecutionOutput,
        ironclaw_host_runtime::RuntimeProcessError,
    > {
        Ok(ironclaw_host_runtime::CommandExecutionOutput {
            output: String::new(),
            saved_output: None,
            exit_code: 0,
            sandboxed: true,
            duration: Duration::ZERO,
        })
    }
}

#[tokio::test]
async fn local_dev_runtime_readiness_reports_trigger_poller_worker() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "trigger readiness".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-trigger-readiness-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-trigger-readiness-tenant".to_string(),
        agent_id: "runtime-trigger-readiness-agent".to_string(),
        source_binding_id: "runtime-trigger-readiness-source".to_string(),
        reply_target_binding_id: "runtime-trigger-readiness-reply".to_string(),
    })
    .with_trigger_poller_settings(
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test(),
    )
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");

    assert!(runtime.readiness().workers.turn_runner);
    assert!(runtime.readiness().workers.trigger_poller);

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_runtime_rejects_trigger_poller_without_creator_authorization() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "trigger auth required".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-trigger-auth-required-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-trigger-auth-required-tenant".to_string(),
        agent_id: "runtime-trigger-auth-required-agent".to_string(),
        source_binding_id: "runtime-trigger-auth-required-source".to_string(),
        reply_target_binding_id: "runtime-trigger-auth-required-reply".to_string(),
    })
    .with_trigger_poller_settings(TriggerPollerSettings::enabled())
    .with_model_gateway_override(gateway);

    let err = match build_reborn_runtime(input).await {
        Ok(runtime) => {
            runtime
                .shutdown()
                .await
                .expect("unexpected runtime shutdown");
            panic!(
                "creator-access-required setting must not enable trigger poller without an access checker"
            );
        }
        Err(err) => err,
    };

    assert!(
        matches!(err, super::RebornRuntimeError::InvalidArgument { reason } if reason.contains("fire-time creator access checker"))
    );
}

#[tokio::test]
async fn local_dev_runtime_accepts_trigger_poller_with_creator_access_checker() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "trigger auth supplied".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-trigger-auth-supplied-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-trigger-auth-supplied-tenant".to_string(),
        agent_id: "runtime-trigger-auth-supplied-agent".to_string(),
        source_binding_id: "runtime-trigger-auth-supplied-source".to_string(),
        reply_target_binding_id: "runtime-trigger-auth-supplied-reply".to_string(),
    })
    .with_trigger_poller_settings(TriggerPollerSettings::enabled())
    .with_trigger_fire_access_checker(Arc::new(AllowingTriggerFireAccessChecker))
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input)
        .await
        .expect("runtime builds with creator access checker");

    assert!(runtime.readiness().workers.turn_runner);
    assert!(runtime.readiness().workers.trigger_poller);

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_runtime_disables_trigger_poller_worker_by_default() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "trigger disabled".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-trigger-disabled-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-trigger-disabled-tenant".to_string(),
        agent_id: "runtime-trigger-disabled-agent".to_string(),
        source_binding_id: "runtime-trigger-disabled-source".to_string(),
        reply_target_binding_id: "runtime-trigger-disabled-reply".to_string(),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");

    assert!(runtime.readiness().workers.turn_runner);
    assert!(!runtime.readiness().workers.trigger_poller);

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_runtime_rejects_invalid_trigger_poller_worker_config() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "trigger invalid config".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let trigger_poller = TriggerPollerSettings::enabled()
        .with_worker_config(
            ironclaw_triggers::TriggerPollerWorkerConfig::default()
                .set_poll_interval(Duration::ZERO),
        )
        .with_tenant_scoped_authorizer_for_test();

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-trigger-invalid-config-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-trigger-invalid-config-tenant".to_string(),
        agent_id: "runtime-trigger-invalid-config-agent".to_string(),
        source_binding_id: "runtime-trigger-invalid-config-source".to_string(),
        reply_target_binding_id: "runtime-trigger-invalid-config-reply".to_string(),
    })
    .with_trigger_poller_settings(trigger_poller)
    .with_model_gateway_override(gateway);

    let err = match build_reborn_runtime(input).await {
        Ok(runtime) => {
            runtime
                .shutdown()
                .await
                .expect("unexpected runtime shutdown");
            panic!("invalid trigger poller config must fail runtime build");
        }
        Err(err) => err,
    };

    assert!(
        matches!(err, super::RebornRuntimeError::InvalidArgument { reason } if reason.contains("poll_interval must be non-zero"))
    );
}

#[tokio::test]
async fn local_dev_runtime_shutdown_cancels_trigger_poller_worker() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "trigger shutdown".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-trigger-shutdown-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-trigger-shutdown-tenant".to_string(),
        agent_id: "runtime-trigger-shutdown-agent".to_string(),
        source_binding_id: "runtime-trigger-shutdown-source".to_string(),
        reply_target_binding_id: "runtime-trigger-shutdown-reply".to_string(),
    })
    .with_trigger_poller_settings(
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test(),
    )
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    assert!(runtime.readiness().workers.trigger_poller);

    tokio::time::timeout(std::time::Duration::from_secs(2), runtime.shutdown())
        .await
        .expect("shutdown returns before timeout")
        .expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_yolo_message_flow_ignores_model_budget_gate() {
    let root = tempfile::tempdir().expect("tempdir");
    let host_home = root.path().join("host-home");
    std::fs::create_dir_all(&host_home).expect("host home");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "yolo budget bypass reply".to_string(),
        requests: Arc::clone(&requests),
    });
    let cost_table = ironclaw_loop_host::StaticModelCostTable::new().with_entry(
        ModelProfileId::new("interactive_model").expect("model profile id"),
        ModelCost {
            input_per_token: dec!(1.00),
            output_per_token: dec!(1.00),
            max_output_tokens: 8_192,
        },
    );

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input_with_profile(
            crate::RebornCompositionProfile::LocalDevYolo,
            "runtime-yolo-budget-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(
            crate::local_dev_yolo_runtime_policy(true).expect("local-yolo policy resolves"),
        )
        .with_local_dev_confirmed_host_home_root(host_home),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-yolo-budget-tenant".to_string(),
        agent_id: "runtime-yolo-budget-agent".to_string(),
        source_binding_id: "runtime-yolo-budget-source".to_string(),
        reply_target_binding_id: "runtime-yolo-budget-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway)
    .with_model_cost_table_override(Arc::new(cost_table));

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Completed);
    assert_eq!(reply.text.as_deref(), Some("yolo budget bypass reply"));
    assert_eq!(
        recorded_request_count(&requests),
        1,
        "local-dev-yolo must reach the model gateway even when a paid cost table is present"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn send_user_message_returns_completed_assistant_text_with_recording_gateway() {
    let root = tempfile::tempdir().expect("tempdir");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "recorded runtime reply".to_string(),
        requests: Arc::clone(&requests),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-success-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-success-tenant".to_string(),
        agent_id: "runtime-success-agent".to_string(),
        source_binding_id: "runtime-success-source".to_string(),
        reply_target_binding_id: "runtime-success-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: RUNTIME_POLL_TIMEOUT,
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Completed);
    assert_eq!(reply.text.as_deref(), Some("recorded runtime reply"));
    assert_eq!(recorded_request_count(&requests), 1);

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn send_user_message_preserves_model_unavailable_after_retry_budget() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(ModelOutageGateway::default());
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-model-outage-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-model-outage-tenant".to_string(),
        agent_id: "runtime-model-outage-agent".to_string(),
        source_binding_id: "runtime-model-outage-source".to_string(),
        reply_target_binding_id: "runtime-model-outage-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: RUNTIME_POLL_TIMEOUT,
    })
    .with_model_gateway_override(gateway.clone())
    // Keep >= 2 retries (the test pins retry-then-fail) but well under
    // the production budget so the deliberate outage fails in seconds.
    .with_model_availability_retry_attempts(std::num::NonZeroU32::new(2).expect("nonzero"));

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "please write a long report"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Failed);
    assert_eq!(reply.failure_category.as_deref(), Some("model_unavailable"));
    assert_eq!(reply.text, None);
    assert!(
        gateway.calls.load(Ordering::SeqCst) >= 3,
        "model outage should be retried before the run fails"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

/// End-to-end Trace Commons auto-capture: a real runtime turn through
/// `send_user_message` must, for an enrolled owner scope, land a redacted
/// envelope in that scope's submission queue without any manual trace
/// command. This drives the full chain: turn completion → lifecycle bus →
/// best-effort capture sink → thread-history read → redact/score →
/// eligibility → queue (+ immediate flush attempt, which fails locally
/// against the closed loopback endpoint and must leave the entry queued).
#[tokio::test]
async fn send_user_message_auto_queues_trace_for_enrolled_scope() {
    use ironclaw_reborn_traces::contribution as trace_contribution;

    let owner = format!("runtime-trace-capture-owner-{}", uuid::Uuid::new_v4());
    // Trace state is keyed by the tenant-scoped composite, so enroll (and
    // later read the queue) under `trace_scope_key(tenant, owner)`, not the
    // bare owner id.
    let scope = trace_contribution::trace_scope_key("runtime-trace-capture-tenant", &owner);
    // Closed loopback port: the immediate flush fails fast and locally; no
    // traffic leaves the machine.
    let policy = trace_contribution::StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_ingestion_endpoint("https://127.0.0.1:1/v1/traces")
        .set_min_submission_score(0.0)
        .set_require_manual_approval_when_pii_detected(false)
        .set_auto_submit_high_value_traces(true);
    trace_contribution::write_trace_policy_for_scope(Some(&scope), &policy)
        .expect("write trace policy");

    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "auto capture reply".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(&owner, root.path().join("local-dev"))
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-trace-capture-tenant".to_string(),
        agent_id: "runtime-trace-capture-agent".to_string(),
        source_binding_id: "runtime-trace-capture-source".to_string(),
        reply_target_binding_id: "runtime-trace-capture-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: RUNTIME_POLL_TIMEOUT,
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "capture this turn"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");
    assert_eq!(reply.status, TurnStatus::Completed);

    // The capture task is detached from the lifecycle path; poll briefly.
    let queue_dir =
        trace_contribution::trace_contribution_dir_for_scope(Some(&scope)).join("queue");
    let queued = |dir: &std::path::Path| -> Vec<std::path::PathBuf> {
        match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .map(|entry| {
                    // Fail loud on a per-entry IO error too, so the test
                    // can't silently drop a broken entry and still claim the
                    // queue holds exactly one envelope.
                    entry
                        .unwrap_or_else(|error| {
                            panic!(
                                "failed to read a trace queue entry in {}: {error}",
                                dir.display()
                            )
                        })
                        .path()
                })
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name.ends_with(".json") && !name.ends_with(".held.json")
                        })
                })
                .collect(),
            // The queue dir not existing yet is the expected pre-capture
            // state; any other IO error is a real failure the test must not
            // mask as "no queued traces".
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => panic!("failed to read trace queue dir {}: {error}", dir.display()),
        }
    };
    let mut entries = Vec::new();
    for _ in 0..150 {
        entries = queued(&queue_dir);
        if !entries.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        entries.len(),
        1,
        "a completed turn for an enrolled scope must auto-queue one trace envelope"
    );
    let body = std::fs::read_to_string(&entries[0]).expect("queued envelope readable");
    let envelope: serde_json::Value = serde_json::from_str(&body).expect("envelope is JSON");
    assert_eq!(envelope["outcome"]["task_success"], "success");

    runtime.shutdown().await.expect("runtime shutdown");
    #[allow(clippy::let_underscore_must_use)] // best-effort per-test scope dir cleanup
    let _ = std::fs::remove_dir_all(trace_contribution::trace_contribution_dir_for_scope(Some(
        &scope,
    )));
}

/// Regression guard: `send_user_message` must persist a
/// `TurnOwner::Personal` (the bound actor user) in `product_context`,
/// not a `TurnOwner::SharedAgent`.  Before the fix, `turn_scope_for`
/// built an ownerless scope whose `product_owner` resolved to
/// `SharedAgent` because `agent_id` was set and no explicit owner was
/// carried.
#[tokio::test(flavor = "multi_thread")]
async fn send_user_message_persists_personal_owner_for_webui() {
    use ironclaw_turns::TurnOwner;

    let root = tempfile::tempdir().expect("tempdir");
    let actor_owner_id = "runtime-personal-owner-user";
    let gateway = Arc::new(RecordingGateway {
        reply: "owner-check reply".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(actor_owner_id, root.path().join("local-dev"))
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-personal-owner-tenant".to_string(),
        agent_id: "runtime-personal-owner-agent".to_string(),
        source_binding_id: "runtime-personal-owner-source".to_string(),
        reply_target_binding_id: "runtime-personal-owner-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: RUNTIME_POLL_TIMEOUT,
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("runtime send should finish within timeout")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Completed);

    // Verify the persisted product_context carries Personal{user: actor_user_id},
    // not SharedAgent.
    let scope = runtime.turn_scope_for(&conversation.0);
    let run_state = runtime
        .turn_coordinator
        .get_run_state(GetRunStateRequest {
            scope,
            run_id: reply.run_id,
        })
        .await
        .expect("get_run_state should succeed");

    let product_context = run_state
        .product_context
        .expect("product_context must be set by send_user_message");
    let expected_user_id = UserId::new(actor_owner_id).expect("actor user id should be valid");
    assert!(
        matches!(
            &product_context.owner,
            TurnOwner::Personal { user } if user == &expected_user_id
        ),
        "send_user_message must persist TurnOwner::Personal{{user: actor_user_id}}, \
             got {:?}",
        product_context.owner
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

/// Regression guard: `send_user_message` resolves product context via
/// `resolve_web_ui`, which sets `TurnOriginKind::WebUi`.  The runtime
/// context section rendered into the model request must therefore contain
/// the WebUI origin line produced by
/// `LoopRuntimeContext::render_model_content`.  Previously, only the
/// persisted `product_context` owner was asserted; this test closes the
/// gap by asserting the *rendered* origin appears in the captured model
/// request.
#[tokio::test]
async fn send_user_message_renders_cli_origin_in_model_request() {
    let root = tempfile::tempdir().expect("tempdir");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "webui-origin-check reply".to_string(),
        requests: Arc::clone(&requests),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-webui-origin-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-webui-origin-tenant".to_string(),
        agent_id: "runtime-webui-origin-agent".to_string(),
        source_binding_id: "runtime-webui-origin-source".to_string(),
        reply_target_binding_id: "runtime-webui-origin-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: RUNTIME_POLL_TIMEOUT,
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("runtime send should finish within timeout")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Completed);

    // The runtime-context system message carries the rendered
    // `LoopRuntimeContext` — its content_ref uses the "runtime" section
    // prefix stamped by `push_runtime_context`.
    let runtime_context_content = {
        let requests = requests
            .lock()
            .expect("recording gateway requests lock poisoned");
        requests[0]
            .messages
            .iter()
            .find(|message| {
                message.role == HostManagedModelMessageRole::System
                    && message
                        .content_ref
                        .as_str()
                        .starts_with("msg:runtime.loop-start.")
            })
            .expect(
                "model request must include a runtime-context system message \
                     (content_ref starts with msg:runtime.loop-start.)",
            )
            .content
            .clone()
    };

    // Exact string produced by LoopRuntimeContext::render_model_content for
    // local runtime chat, which stamps the first-party source channel as CLI.
    assert!(
        runtime_context_content.contains("Run origin: CLI chat; replies render in this session."),
        "runtime-context system message must contain the CLI origin line, \
             got: {runtime_context_content:?}"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn send_user_message_until_gate_returns_blocked_on_auth_gate() {
    let root = tempfile::tempdir().expect("tempdir");
    let host_home = root.path().join("host-home");
    std::fs::create_dir_all(&host_home).expect("host home");
    let gateway = Arc::new(AuthGateToolCallingGateway::default());
    let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "runtime-auth-gate-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(
            crate::local_dev_yolo_runtime_policy(true).expect("local-yolo policy resolves"),
        )
        .with_local_dev_confirmed_host_home_root(host_home),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-auth-gate-tenant".to_string(),
        agent_id: "runtime-auth-gate-agent".to_string(),
        source_binding_id: "runtime-auth-gate-source".to_string(),
        reply_target_binding_id: "runtime-auth-gate-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: RUNTIME_POLL_TIMEOUT,
    })
    .with_model_gateway_override(gateway_for_runtime);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let extension_management = &runtime.extension_management;
    let notion_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion")
        .expect("valid notion ref");
    // #6520 removed the port-side operator accessor: install as the runtime
    // owner (the tenant operator this runtime was constructed with).
    let operator = UserId::new("runtime-auth-gate-owner").expect("valid lifecycle caller");
    extension_management
        .install(notion_ref.clone(), &operator)
        .await
        .expect("install Notion MCP");
    // v3 hosted-MCP packages publish no model-visible tools on static
    // activation; script tools/list discovery so the notion-search tool
    // the auth-gate gateway calls exists as a model-visible capability.
    extension_management
        .activate_with_prechecked_credentials_for_test(
            notion_ref,
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: ResourceScope::local_default(operator.clone(), InvocationId::new())
                    .expect("valid scope"),
                runtime_http_egress: Arc::new(
                    crate::extension_host::extension_lifecycle::hosted_mcp_test_support::HostedMcpDiscoveryEgress::with_tool_name("notion-search"),
                ),
            },
            &operator,
        )
        .await
        .expect("activate Notion MCP with scripted discovery");

    // #6520 caller-phase surface: notion tools are model-visible only when the
    // caller's notion credential account resolves. Seed a Configured account
    // whose secret handle has no stored material — the surface becomes
    // visible while dispatch-time injection still raises the auth gate this
    // test drives.
    {
        use ironclaw_auth::{
            AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel,
            CredentialAccountStatus, CredentialOwnership, NewCredentialAccount,
        };
        let mut scope = runtime.thread_scope.to_resource_scope();
        scope.user_id = runtime.actor_user_id.clone();
        scope.mission_id = None;
        scope.thread_id = None;
        scope.invocation_id = InvocationId::new();
        runtime
            .product_auth
            .credential_account_service()
            .create_account(NewCredentialAccount {
                scope: AuthProductScope::credential_owner(&scope, AuthSurface::Api),
                provider: AuthProviderId::new("notion").expect("provider"),
                label: CredentialAccountLabel::new("notion").expect("label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(
                    ironclaw_host_api::SecretHandle::new("notion-test-token".to_string())
                        .expect("secret handle"),
                ),
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .expect("create configured notion account without secret material");
    }

    let conversation = runtime.new_conversation().await.expect("conversation");
    runtime
        .enable_global_auto_approve_for_test(&conversation)
        .await;
    let outcome = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message_until_gate(&conversation, "search Notion"),
    )
    .await
    .expect("gate-aware send should return before timeout")
    .expect("gate-aware send should succeed");

    let (run_id, gate_ref) = match outcome {
        super::RebornTurnDriveOutcome::BlockedOnGate {
            run_id,
            status,
            gate_ref,
            ..
        } => {
            assert_eq!(status, TurnStatus::BlockedAuth);
            assert!(
                gate_ref.as_str().starts_with("gate:auth-"),
                "auth gate ref should carry the auth prefix, got {}",
                gate_ref.as_str()
            );
            (run_id, gate_ref)
        }
        super::RebornTurnDriveOutcome::Terminal(reply) => {
            panic!("auth-gated turn should pause before terminal reply, got {reply:?}");
        }
    };
    let state = runtime
        .turn_coordinator
        .get_run_state(GetRunStateRequest {
            scope: runtime.turn_scope_for(&conversation.0),
            run_id,
        })
        .await
        .expect("blocked run state");
    assert_eq!(state.status, TurnStatus::BlockedAuth);
    assert_eq!(state.gate_ref.as_ref(), Some(&gate_ref));

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn cancel_run_propagates_to_subagent_children() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "unused".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-cancel-child-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-cancel-child-tenant".to_string(),
        agent_id: "runtime-cancel-child-agent".to_string(),
        source_binding_id: "runtime-cancel-child-source".to_string(),
        reply_target_binding_id: "runtime-cancel-child-reply".to_string(),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    stop_turn_runner_worker_for_manual_state_test(&runtime).await;
    let conversation = runtime.new_conversation().await.expect("conversation");
    let parent_scope = runtime.turn_scope_for(&conversation.0);
    let actor = TurnActor::new(runtime.actor_user_id.clone());
    let parent = runtime
        .turn_coordinator
        .submit_turn(SubmitTurnRequest {
            requested_model: None,
            scope: parent_scope.clone(),
            actor: actor.clone(),
            accepted_message_ref: AcceptedMessageRef::new("msg:cancel-parent").unwrap(),
            source_binding_ref: SourceBindingRef::new("source:cancel-parent").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:cancel-parent").unwrap(),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new("cancel-parent").unwrap(),
            received_at: Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
        })
        .await
        .expect("parent submitted");
    let SubmitTurnResponse::Accepted {
        run_id: parent_run_id,
        ..
    } = parent;
    let child_scope = TurnScope::new(
        parent_scope.tenant_id.clone(),
        parent_scope.agent_id.clone(),
        parent_scope.project_id.clone(),
        ThreadId::new("runtime-cancel-child-thread").unwrap(),
    );
    let child = runtime
        .turn_tree_store
        .submit_child_turn(
            SubmitChildRunRequest {
                parent_scope: parent_scope.clone(),
                parent_run_id,
                child_scope: child_scope.clone(),
                actor,
                accepted_message_ref: AcceptedMessageRef::new("msg:cancel-child").unwrap(),
                source_binding_ref: SourceBindingRef::new("source:cancel-child").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:cancel-child").unwrap(),
                requested_run_profile: None,
                idempotency_key: IdempotencyKey::new("cancel-child").unwrap(),
                received_at: Utc::now(),
                requested_run_id: None,
                spawn_tree_descendant_cap: 4,
            },
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .expect("child submitted");
    let SubmitTurnResponse::Accepted {
        run_id: child_run_id,
        ..
    } = child;

    runtime
        .cancel_run(
            &parent_scope,
            parent_run_id,
            SanitizedCancelReason::UserRequested,
            "test-parent-cancel",
        )
        .await
        .expect("parent cancellation succeeds");

    let result_ref = LoopResultRef::new("result:runtime-cancel-child").unwrap();
    let parent_resolved_run_profile = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .expect("resolve run profile");
    let parent_run_context = LoopRunContext::new(
        parent_scope.clone(),
        TurnId::new(),
        parent_run_id,
        parent_resolved_run_profile,
    );
    runtime
        .thread_service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            scope: runtime.thread_scope.clone(),
            thread_id: parent_scope.thread_id.clone(),
            turn_run_id: parent_run_id.to_string(),
            result_ref: result_ref.as_str().to_string(),
            safe_summary: ToolResultSafeSummary::new("subagent spawned").unwrap(),
            provider_call: None,
            model_observation: None,
        })
        .await
        .expect("parent result reference seeded");
    let child_thread_scope = ThreadScope {
        tenant_id: child_scope.tenant_id.clone(),
        agent_id: child_scope.agent_id.clone().unwrap(),
        project_id: child_scope.project_id.clone(),
        owner_user_id: Some(runtime.actor_user_id.clone()),
        mission_id: None,
    };
    runtime
        .thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: child_thread_scope,
            thread_id: Some(child_scope.thread_id.clone()),
            created_by_actor_id: "test".to_string(),
            title: Some("Subagent".to_string()),
            metadata_json: Some(
                serde_json::to_string(&SubagentThreadMetadata {
                    kind: SubagentThreadKind::Subagent,
                    parent_run_id,
                    parent_thread_id: parent_scope.thread_id.clone(),
                    tree_root_run_id: parent_run_id,
                    child_run_id,
                    subagent_kind: SubagentKindId::new("general").unwrap(),
                    mode: SpawnSubagentMode::Blocking,
                    result_ref,
                    handoff: None,
                    parent_run_context: parent_run_context.clone(),
                    gate_ref: ironclaw_turns::GateRef::new("gate:runtime-cancel-child").unwrap(),
                })
                .unwrap(),
            ),
        })
        .await
        .expect("child thread metadata seeded");

    let child_state = runtime
        .turn_coordinator
        .get_run_state(GetRunStateRequest {
            scope: child_scope,
            run_id: child_run_id,
        })
        .await
        .expect("child state");
    assert_eq!(child_state.status, TurnStatus::Cancelled);

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn send_user_message_uses_caller_supplied_skill_context_source() {
    let root = tempfile::tempdir().expect("tempdir");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "should not reach model".to_string(),
        requests: Arc::clone(&requests),
    });
    let skill_context_source = Arc::new(FailingSkillContextSource::default());
    let skill_context_source_for_input: Arc<dyn HostSkillContextSource> =
        skill_context_source.clone();
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-skill-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-skill-tenant".to_string(),
        agent_id: "runtime-skill-agent".to_string(),
        source_binding_id: "runtime-skill-source".to_string(),
        reply_target_binding_id: "runtime-skill-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_skill_context_source(skill_context_source_for_input)
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_ne!(reply.status, TurnStatus::Completed);
    assert_eq!(
        skill_context_source.calls.load(Ordering::SeqCst),
        1,
        "composition should pass caller-supplied skill context into the planned runtime"
    );
    assert!(
        requests
            .lock()
            .expect("recording gateway requests lock poisoned")
            .is_empty(),
        "skill context failure should stop before model dispatch"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_runtime_exposes_host_runtime_capabilities_to_model_calls() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(ToolCallingGateway::default());
    let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-tools-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-tools-tenant".to_string(),
        agent_id: "runtime-tools-agent".to_string(),
        source_binding_id: "runtime-tools-source".to_string(),
        reply_target_binding_id: "runtime-tools-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway_for_runtime);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    runtime
        .enable_global_auto_approve_for_test(&conversation)
        .await;
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "use echo tool"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Completed, "reply: {reply:?}");
    assert_eq!(reply.text.as_deref(), Some("tool ok"));
    assert_eq!(
        *gateway
            .stream_model_calls
            .lock()
            .expect("tool gateway stream count lock poisoned"),
        0,
        "runtime should use capability-aware model path"
    );
    assert_eq!(
        gateway
            .requests
            .lock()
            .expect("tool gateway requests lock poisoned")
            .len(),
        2,
        "tool call should require initial request plus tool-result follow-up"
    );
    let history = runtime
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: runtime.thread_scope.clone(),
            thread_id: conversation.0.clone(),
        })
        .await
        .expect("thread history");
    let tool_result = history
        .messages
        .iter()
        .find(|message| message.kind == MessageKind::ToolResultReference)
        .expect("tool result reference should persist in thread history");
    assert!(
        tool_result
            .tool_result_ref
            .as_deref()
            .is_some_and(|result_ref| result_ref.starts_with("result:")),
        "tool result should persist a durable result ref"
    );
    assert!(
        tool_result.tool_result_provider_call.is_none(),
        "product thread history should scrub provider replay metadata"
    );
    let context = runtime
        .thread_service
        .load_context_messages(LoadContextMessagesRequest {
            scope: runtime.thread_scope.clone(),
            thread_id: conversation.0.clone(),
            message_ids: vec![tool_result.message_id],
        })
        .await
        .expect("tool result context");
    let provider_call = context.messages[0]
        .tool_result_provider_call
        .as_ref()
        .expect("model context should preserve provider replay metadata");
    assert_eq!(provider_call.provider_call_id, "call-1");
    assert_eq!(
        provider_call.capability_id,
        CapabilityId::new("builtin.echo").unwrap()
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

/// Records both trajectory callbacks so the e2e test can assert the
/// observer fires through a real `build_reborn_runtime` turn — driving the
/// input hook (`HostRuntimeLoopCapabilityPort`) and the result hook
/// (`StagedCapabilityIo::write_capability_result`) on the actual dispatch
/// path, not a direct helper call.
#[derive(Debug, Default)]
struct RecordingTrajectoryObserver {
    inputs: StdMutex<Vec<(String, String, serde_json::Value)>>,
    results: StdMutex<Vec<(String, String, serde_json::Value)>>,
}

impl crate::RebornTrajectoryObserver for RecordingTrajectoryObserver {
    fn on_capability_input(
        &self,
        call_id: &str,
        capability_id: &str,
        arguments: &serde_json::Value,
    ) {
        self.inputs.lock().expect("inputs lock").push((
            call_id.to_string(),
            capability_id.to_string(),
            arguments.clone(),
        ));
    }

    fn on_capability_result(&self, call_id: &str, capability_id: &str, output: &serde_json::Value) {
        self.results.lock().expect("results lock").push((
            call_id.to_string(),
            capability_id.to_string(),
            output.clone(),
        ));
    }
}

/// End-to-end guard for the #4588 trajectory observer seam: a real runtime
/// turn that dispatches the `builtin.echo` capability must fire BOTH the
/// input and result callbacks installed via
/// `RebornRuntimeInput::with_raw_trajectory_observer`. This drives the
/// result hook on the genuine dispatch path (the prior direct-call unit
/// test was dropped as false confidence — it stayed green even when
/// end-to-end dispatch was broken).
#[tokio::test]
async fn local_dev_runtime_forwards_tool_call_trajectory_to_raw_observer() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(ToolCallingGateway::default());
    let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
    let observer = Arc::new(RecordingTrajectoryObserver::default());
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-trajectory-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-trajectory-tenant".to_string(),
        agent_id: "runtime-trajectory-agent".to_string(),
        source_binding_id: "runtime-trajectory-source".to_string(),
        reply_target_binding_id: "runtime-trajectory-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    // Raw (not safe-preview) so we can assert verbatim arguments + output.
    .with_raw_trajectory_observer(observer.clone())
    .with_model_gateway_override(gateway_for_runtime);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    runtime
        .enable_global_auto_approve_for_test(&conversation)
        .await;
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "use echo tool"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");
    assert_eq!(reply.status, TurnStatus::Completed, "reply: {reply:?}");
    // Shut down before inspecting the recorded callbacks so the std-Mutex
    // guards are never held across an `.await` (clippy::await_holding_lock).
    runtime.shutdown().await.expect("runtime shutdown");

    let echo_id = CapabilityId::new("builtin.echo").unwrap();

    let inputs = observer.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1, "exactly one capability input observed");
    let (input_call_id, input_capability, arguments) = &inputs[0];
    assert!(!input_call_id.is_empty(), "input call_id should be present");
    assert_eq!(input_capability, echo_id.as_str());
    assert_eq!(
        arguments,
        &serde_json::json!({"message": "hello from tool"}),
        "observer should receive the raw model-emitted tool arguments"
    );

    let results = observer.results.lock().expect("results lock");
    assert_eq!(results.len(), 1, "exactly one capability result observed");
    let (result_call_id, result_capability, output) = &results[0];
    assert_eq!(result_capability, echo_id.as_str());
    assert_eq!(
        result_call_id, input_call_id,
        "result and input callbacks correlate by call_id"
    );
    assert!(
        output.to_string().contains("hello from tool"),
        "observer should receive the staged capability output, got {output}"
    );
}

/// Caller-level guard for the **default** (safe-preview) observer path:
/// installing via the public `with_trajectory_observer` and driving a real
/// turn with a large tool payload must deliver a *bounded* preview to the
/// observer — proving truncation is wired between dispatch and the observer,
/// not just unit-tested on the helper in isolation.
#[tokio::test]
async fn local_dev_runtime_safe_preview_observer_receives_bounded_payload() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(LargeEchoToolCallingGateway::default());
    let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
    let observer = Arc::new(RecordingTrajectoryObserver::default());
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-preview-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-preview-tenant".to_string(),
        agent_id: "runtime-preview-agent".to_string(),
        source_binding_id: "runtime-preview-source".to_string(),
        reply_target_binding_id: "runtime-preview-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    // Default path → safe-preview truncation applied before the observer.
    .with_trajectory_observer(observer.clone())
    .with_model_gateway_override(gateway_for_runtime);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    runtime
        .enable_global_auto_approve_for_test(&conversation)
        .await;
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "echo a big payload"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");
    assert_eq!(reply.status, TurnStatus::Completed, "reply: {reply:?}");
    // Shut down before inspecting the recorded callbacks so the std-Mutex
    // guards are never held across an `.await` (clippy::await_holding_lock).
    runtime.shutdown().await.expect("runtime shutdown");

    let original_len = large_echo_message().len();

    let inputs = observer.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 2, "echo and result_read inputs observed");
    let observed_message = inputs[0].2["message"].as_str().expect("message string");
    assert!(
        observed_message.len() < original_len && observed_message.contains("[truncated"),
        "observer should receive a truncated preview of the large argument, got {} bytes",
        observed_message.len()
    );
    assert_eq!(inputs[1].1, "builtin.result_read");

    let results = observer.results.lock().expect("results lock");
    assert_eq!(results.len(), 2, "echo and result_read outputs observed");
    assert!(
        results[0].2.to_string().contains("[truncated"),
        "observer should receive a truncated preview of the large result"
    );
    assert_eq!(results[1].1, "builtin.result_read");
}

#[tokio::test]
async fn local_dev_runtime_wires_input_skill_context_source_to_model_calls() {
    let root = tempfile::tempdir().expect("tempdir");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "skill context ok".to_string(),
        requests: Arc::clone(&requests),
    });
    let skill_source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::loaded(
            skill_md(
                "review-helper",
                "review helper description",
                "Use review helper prompt content.",
            ),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        ),
    ]));
    let skill_context_source: Arc<dyn HostSkillContextSource> = skill_source;
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-skill-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-skill-tenant".to_string(),
        agent_id: "runtime-skill-agent".to_string(),
        source_binding_id: "runtime-skill-source".to_string(),
        reply_target_binding_id: "runtime-skill-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_skill_context_source(skill_context_source)
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "review this"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Completed);
    assert_eq!(reply.text.as_deref(), Some("skill context ok"));
    let (request_count, skill_message_content) = {
        let requests = requests
            .lock()
            .expect("recording gateway requests lock poisoned");
        let skill_message = requests[0]
            .messages
            .iter()
            .find(|message| {
                message.role == HostManagedModelMessageRole::System
                    && message
                        .content_ref
                        .as_str()
                        .starts_with("msg:snippet.skill.review-helper.")
            })
            .expect("model request should include skill-context system message");
        (requests.len(), skill_message.content.clone())
    };
    assert_eq!(request_count, 1);
    assert!(skill_message_content.contains("review helper description"));
    assert!(skill_message_content.contains("Use review helper prompt content."));

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_runtime_prefers_configured_skill_context_source_over_filesystem_default() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/skills/filesystem-helper"))
        .expect("filesystem skill dir");
    std::fs::write(
        storage_root.join("system/skills/filesystem-helper/SKILL.md"),
        skill_md(
            "filesystem-helper",
            "filesystem helper description",
            "FILESYSTEM_HELPER_PROMPT_SENTINEL",
        ),
    )
    .expect("write filesystem skill");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "configured skill context ok".to_string(),
        requests: Arc::clone(&requests),
    });
    let skill_source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::loaded(
            skill_md(
                "configured-helper",
                "configured helper description",
                "CONFIGURED_HELPER_PROMPT_SENTINEL",
            ),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        ),
    ]));
    let skill_context_source: Arc<dyn HostSkillContextSource> = skill_source;
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input("runtime-skill-override-owner", storage_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-skill-override-tenant".to_string(),
        agent_id: "runtime-skill-override-agent".to_string(),
        source_binding_id: "runtime-skill-override-source".to_string(),
        reply_target_binding_id: "runtime-skill-override-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_skill_context_source(skill_context_source)
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "review this"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Completed);
    assert_eq!(reply.text.as_deref(), Some("configured skill context ok"));
    let combined_skill_context = {
        let requests = requests
            .lock()
            .expect("recording gateway requests lock poisoned");
        requests[0]
            .messages
            .iter()
            .filter(|message| {
                message.role == HostManagedModelMessageRole::System
                    && message
                        .content_ref
                        .as_str()
                        .starts_with("msg:snippet.skill.")
            })
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(combined_skill_context.contains("configured helper description"));
    assert!(combined_skill_context.contains("CONFIGURED_HELPER_PROMPT_SENTINEL"));
    assert!(!combined_skill_context.contains("filesystem helper description"));
    assert!(!combined_skill_context.contains("FILESYSTEM_HELPER_PROMPT_SENTINEL"));

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_runtime_wires_filesystem_skills_by_default_to_model_calls() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/skills/system-helper"))
        .expect("system skill dir");
    std::fs::write(
        storage_root.join("system/skills/system-helper/SKILL.md"),
        skill_md(
            "system-helper",
            "system helper description",
            "SYSTEM_HELPER_PROMPT_SENTINEL",
        ),
    )
    .expect("write system skill");
    let local_helper_dir = user_skill_dir(
        &storage_root,
        "runtime-filesystem-skill-tenant",
        "runtime-filesystem-skill-owner",
        "local-helper",
    );
    std::fs::create_dir_all(&local_helper_dir).expect("user skill dir");
    std::fs::write(
        local_helper_dir.join("SKILL.md"),
        skill_md(
            "local-helper",
            "local helper description",
            "USER_HELPER_PROMPT_SENTINEL",
        ),
    )
    .expect("write user skill");
    std::fs::create_dir_all(storage_root.join("tenant-shared/skills/shared-helper"))
        .expect("tenant shared skill dir");
    std::fs::write(
        storage_root.join("tenant-shared/skills/shared-helper/SKILL.md"),
        skill_md(
            "shared-helper",
            "tenant shared helper description",
            "TENANT_SHARED_PROMPT_SENTINEL",
        ),
    )
    .expect("write tenant shared skill");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "filesystem skill context ok".to_string(),
        requests: Arc::clone(&requests),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input("runtime-filesystem-skill-owner", storage_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-filesystem-skill-tenant".to_string(),
        agent_id: "runtime-filesystem-skill-agent".to_string(),
        source_binding_id: "runtime-filesystem-skill-source".to_string(),
        reply_target_binding_id: "runtime-filesystem-skill-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: RUNTIME_POLL_TIMEOUT,
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "/system-helper and /local-helper"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Completed);
    assert_eq!(reply.text.as_deref(), Some("filesystem skill context ok"));
    let skill_messages = {
        let requests = requests
            .lock()
            .expect("recording gateway requests lock poisoned");
        requests[0]
            .messages
            .iter()
            .filter(|message| {
                message.role == HostManagedModelMessageRole::System
                    && message
                        .content_ref
                        .as_str()
                        .starts_with("msg:snippet.skill.")
            })
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
    };
    let combined_skill_context = skill_messages.join("\n");
    // Default `listing` injection: the two explicitly-mentioned skills load
    // their full bodies, and every other visible skill (the bundled system
    // skills) collapses into ONE `available-skills` listing message.
    assert_eq!(skill_messages.len(), 3);
    assert!(combined_skill_context.contains("system helper description"));
    assert!(combined_skill_context.contains("SYSTEM_HELPER_PROMPT_SENTINEL"));
    assert!(combined_skill_context.contains("local helper description"));
    assert!(combined_skill_context.contains("USER_HELPER_PROMPT_SENTINEL"));
    assert!(!combined_skill_context.contains("tenant shared helper description"));
    assert!(!combined_skill_context.contains("TENANT_SHARED_PROMPT_SENTINEL"));
    assert!(
        combined_skill_context.contains("builtin.skill_activate"),
        "available-skills listing message must reach the model"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_runtime_backfills_legacy_owner_skill_root() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("skills/legacy-helper")).expect("legacy skill dir");
    std::fs::write(
        storage_root.join("skills/legacy-helper/SKILL.md"),
        skill_md(
            "legacy-helper",
            "legacy helper description",
            "LEGACY_HELPER_PROMPT_SENTINEL",
        ),
    )
    .expect("write legacy helper skill");

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-legacy-skill-owner",
            storage_root.clone(),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    );
    let runtime = build_reborn_runtime(input).await.expect("runtime");
    let conversation = runtime.new_conversation().await.expect("conversation");

    let result = runtime
        .execute_skill_message(&conversation, "$legacy-helper")
        .await
        .expect("execute skill message");

    assert_eq!(result.plan.activations().len(), 1);
    assert_eq!(result.plan.activations()[0].name, "legacy-helper");
    assert!(
        storage_root
            .join(
                "tenants/reborn-cli/users/runtime-legacy-skill-owner/skills/legacy-helper/SKILL.md"
            )
            .exists()
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn execute_skill_message_returns_plan_and_reads_active_bundle_assets() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let asset_helper_dir = user_skill_dir(
        &storage_root,
        "runtime-skill-exec-tenant",
        "runtime-skill-exec-owner",
        "asset-helper",
    );
    std::fs::create_dir_all(asset_helper_dir.join("references"))
        .expect("asset skill references dir");
    std::fs::write(
        asset_helper_dir.join("SKILL.md"),
        skill_md(
            "asset-helper",
            "asset helper description",
            "ASSET_HELPER_PROMPT_SENTINEL",
        ),
    )
    .expect("write asset helper skill");
    std::fs::write(
        asset_helper_dir.join("references/policy.md"),
        "asset helper policy",
    )
    .expect("write asset helper policy");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "asset helper ok".to_string(),
        requests: Arc::clone(&requests),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input("runtime-skill-exec-owner", storage_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-skill-exec-tenant".to_string(),
        agent_id: "runtime-skill-exec-agent".to_string(),
        source_binding_id: "runtime-skill-exec-source".to_string(),
        reply_target_binding_id: "runtime-skill-exec-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let result = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.execute_skill_message(&conversation, "$asset-helper use policy"),
    )
    .await
    .expect("skill execution should finish")
    .expect("skill execution should succeed");

    assert_eq!(result.reply.status, TurnStatus::Completed);
    assert_eq!(result.reply.text.as_deref(), Some("asset helper ok"));
    assert_eq!(result.plan.activations().len(), 1);
    assert_eq!(result.plan.activations()[0].name, "asset-helper");
    assert_eq!(
        result.plan.activations()[0].source,
        Some(RebornSkillSourceKind::User)
    );
    assert_eq!(result.plan.active_bundles().len(), 1);
    assert_eq!(result.plan.active_bundles()[0].skill_name, "asset-helper");
    assert_eq!(
        result.plan.run_context().run_id,
        result.reply.run_id,
        "post-activation asset reads must reuse the real activation run context"
    );
    let asset = runtime
        .read_skill_execution_asset(
            &conversation,
            &result.plan,
            &result.plan.activations()[0],
            "references/policy.md",
        )
        .await
        .expect("active bundle asset read succeeds");

    assert_eq!(asset.skill_name, "asset-helper");
    assert_eq!(asset.path, "references/policy.md");
    assert_eq!(asset.into_utf8().unwrap(), "asset helper policy");

    let other_conversation = runtime
        .new_conversation()
        .await
        .expect("other conversation");
    let error = runtime
        .read_skill_execution_asset(
            &other_conversation,
            &result.plan,
            &result.plan.activations()[0],
            "references/policy.md",
        )
        .await
        .expect_err("plan should be bound to its activation conversation");
    assert!(
        error
            .to_string()
            .contains("skill execution plan does not belong to this conversation"),
        "unexpected error: {error}"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_runtime_fails_closed_for_ambiguous_explicit_skill_before_model_call() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/skills/code-review"))
        .expect("system skill dir");
    std::fs::write(
        storage_root.join("system/skills/code-review/SKILL.md"),
        skill_md(
            "code-review",
            "system review description",
            "SYSTEM_REVIEW_PROMPT_SENTINEL",
        ),
    )
    .expect("write system skill");
    let user_code_review_dir = user_skill_dir(
        &storage_root,
        "runtime-ambiguous-skill-tenant",
        "runtime-ambiguous-skill-owner",
        "code-review",
    );
    std::fs::create_dir_all(&user_code_review_dir).expect("user skill dir");
    std::fs::write(
        user_code_review_dir.join("SKILL.md"),
        skill_md(
            "code-review",
            "user review description",
            "USER_REVIEW_PROMPT_SENTINEL",
        ),
    )
    .expect("write user skill");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "should not reach model".to_string(),
        requests: Arc::clone(&requests),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input("runtime-ambiguous-skill-owner", storage_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-ambiguous-skill-tenant".to_string(),
        agent_id: "runtime-ambiguous-skill-agent".to_string(),
        source_binding_id: "runtime-ambiguous-skill-source".to_string(),
        reply_target_binding_id: "runtime-ambiguous-skill-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: RUNTIME_POLL_TIMEOUT,
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "/code-review this PR"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_ne!(reply.status, TurnStatus::Completed);
    assert!(
        requests
            .lock()
            .expect("recording gateway requests lock poisoned")
            .is_empty(),
        "ambiguous explicit skill should fail before model dispatch"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_runtime_suppresses_explicit_setup_skill_when_workspace_marker_exists() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let marker_helper_dir = user_skill_dir(
        &storage_root,
        "runtime-setup-marker-tenant",
        "runtime-setup-marker-owner",
        "marker-helper",
    );
    std::fs::create_dir_all(&marker_helper_dir).expect("user skill dir");
    std::fs::create_dir_all(storage_root.join("workspace/markers")).expect("marker dir");
    std::fs::write(
        marker_helper_dir.join("SKILL.md"),
        skill_md_with_setup_marker(
            "marker-helper",
            "marker helper description",
            "markers/marker-helper.done",
            "MARKER_HELPER_PROMPT_SENTINEL",
        ),
    )
    .expect("write marker helper skill");
    std::fs::write(
        storage_root.join("workspace/markers/marker-helper.done"),
        "done",
    )
    .expect("write setup marker");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "setup marker ok".to_string(),
        requests: Arc::clone(&requests),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input("runtime-setup-marker-owner", storage_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-setup-marker-tenant".to_string(),
        agent_id: "runtime-setup-marker-agent".to_string(),
        source_binding_id: "runtime-setup-marker-source".to_string(),
        reply_target_binding_id: "runtime-setup-marker-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: RUNTIME_POLL_TIMEOUT,
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let result = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.execute_skill_message(&conversation, "$marker-helper"),
    )
    .await
    .expect("skill execution should finish")
    .expect("skill execution should succeed");

    assert_eq!(result.reply.status, TurnStatus::Completed);
    assert!(result.plan.activations().is_empty());
    // The setup skill's body must not reach the model when its marker is
    // already satisfied. The always-present one-line available-skills
    // listing snippet (`msg:snippet.skill.available-skills.*`) may still
    // advertise the skill's short description, but the full SKILL.md body —
    // pinned by MARKER_HELPER_PROMPT_SENTINEL — only ships on activation.
    let skill_context = {
        let requests = requests
            .lock()
            .expect("recording gateway requests lock poisoned");
        requests[0]
            .messages
            .iter()
            .filter(|message| {
                message.role == HostManagedModelMessageRole::System
                    && message
                        .content_ref
                        .as_str()
                        .starts_with("msg:snippet.skill.")
            })
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        !skill_context.contains("MARKER_HELPER_PROMPT_SENTINEL"),
        "suppressed setup skill body must not be injected, got: {skill_context}"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_runtime_activates_setup_skill_when_workspace_marker_is_absent() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let marker_helper_dir = user_skill_dir(
        &storage_root,
        "runtime-setup-marker-absent-tenant",
        "runtime-setup-marker-absent-owner",
        "marker-helper",
    );
    std::fs::create_dir_all(&marker_helper_dir).expect("user skill dir");
    std::fs::write(
        marker_helper_dir.join("SKILL.md"),
        skill_md_with_setup_marker(
            "marker-helper",
            "marker helper description",
            "markers/marker-helper.done",
            "MARKER_HELPER_PROMPT_SENTINEL",
        ),
    )
    .expect("write marker helper skill");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "setup marker absent ok".to_string(),
        requests: Arc::clone(&requests),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input("runtime-setup-marker-absent-owner", storage_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-setup-marker-absent-tenant".to_string(),
        agent_id: "runtime-setup-marker-absent-agent".to_string(),
        source_binding_id: "runtime-setup-marker-absent-source".to_string(),
        reply_target_binding_id: "runtime-setup-marker-absent-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let result = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.execute_skill_message(&conversation, "$marker-helper"),
    )
    .await
    .expect("skill execution should finish")
    .expect("skill execution should succeed");

    assert_eq!(result.reply.status, TurnStatus::Completed);
    assert_eq!(result.plan.activations().len(), 1);
    assert_eq!(result.plan.activations()[0].name, "marker-helper");
    let skill_context = {
        let requests = requests
            .lock()
            .expect("recording gateway requests lock poisoned");
        requests[0]
            .messages
            .iter()
            .filter(|message| {
                message.role == HostManagedModelMessageRole::System
                    && message
                        .content_ref
                        .as_str()
                        .starts_with("msg:snippet.skill.")
            })
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(skill_context.contains("marker helper description"));
    assert!(skill_context.contains("MARKER_HELPER_PROMPT_SENTINEL"));

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_runtime_rejects_workspace_overlapping_default_skill_roots() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let workspace_root = storage_root.join("skills");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "should not build".to_string(),
        requests,
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input("runtime-overlap-owner", storage_root)
            .with_local_dev_workspace_root(workspace_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-overlap-tenant".to_string(),
        agent_id: "runtime-overlap-agent".to_string(),
        source_binding_id: "runtime-overlap-source".to_string(),
        reply_target_binding_id: "runtime-overlap-reply".to_string(),
    })
    .with_model_gateway_override(gateway);

    let error = match build_reborn_runtime(input).await {
        Ok(runtime) => {
            runtime.shutdown().await.expect("runtime shutdown");
            panic!("overlapping workspace and skill roots should fail closed");
        }
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("must not overlap default skill root /skills"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn local_dev_runtime_skips_invalid_filesystem_skill_before_model_call() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let bad_helper_dir = user_skill_dir(
        &storage_root,
        "runtime-bad-skill-tenant",
        "runtime-bad-skill-owner",
        "bad-helper",
    );
    std::fs::create_dir_all(&bad_helper_dir).expect("bad skill dir");
    std::fs::write(
        bad_helper_dir.join("SKILL.md"),
        skill_md(
            "different-name",
            "bad helper description",
            "BAD_HELPER_PROMPT_SENTINEL",
        ),
    )
    .expect("write bad skill");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "invalid skill skipped".to_string(),
        requests: Arc::clone(&requests),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input("runtime-bad-skill-owner", storage_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-bad-skill-tenant".to_string(),
        agent_id: "runtime-bad-skill-agent".to_string(),
        source_binding_id: "runtime-bad-skill-source".to_string(),
        reply_target_binding_id: "runtime-bad-skill-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "hello with no matching skill"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Completed);
    assert_eq!(reply.text.as_deref(), Some("invalid skill skipped"));
    let combined_request_content = requests
        .lock()
        .expect("recording gateway requests lock poisoned")
        .iter()
        .flat_map(|request| request.messages.iter())
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!combined_request_content.contains("BAD_HELPER_PROMPT_SENTINEL"));

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_runtime_maps_workspace_to_configured_root() {
    let root = tempfile::tempdir().expect("tempdir");
    let workspace_root = tempfile::tempdir().expect("workspace tempdir");
    std::fs::write(
        workspace_root.path().join("workspace-sentinel.txt"),
        "visible through /workspace",
    )
    .expect("write sentinel");
    let gateway = Arc::new(WorkspaceListingGateway::default());
    let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-workspace-owner",
            root.path().join("local-dev"),
        )
        .with_local_dev_workspace_root(workspace_root.path().to_path_buf())
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-workspace-tenant".to_string(),
        agent_id: "runtime-workspace-agent".to_string(),
        source_binding_id: "runtime-workspace-source".to_string(),
        reply_target_binding_id: "runtime-workspace-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: RUNTIME_POLL_TIMEOUT,
    })
    .with_model_gateway_override(gateway_for_runtime);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    runtime
        .enable_global_auto_approve_for_test(&conversation)
        .await;
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "list workspace"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Completed, "reply: {reply:?}");
    assert_eq!(reply.text.as_deref(), Some("workspace ok"));
    let request_count = {
        let requests = gateway
            .requests
            .lock()
            .expect("workspace gateway requests lock poisoned");
        requests.len()
    };
    assert_eq!(
        request_count, 2,
        "workspace listing should require initial request plus tool-result follow-up"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_runtime_webui_bundle_reuses_thread_and_turn_facades() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "webui projection ok".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-webui-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-webui-tenant".to_string(),
        agent_id: "runtime-webui-agent".to_string(),
        source_binding_id: "runtime-webui-source".to_string(),
        reply_target_binding_id: "runtime-webui-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let runtime_turn_coordinator = runtime.product_turn_coordinator();
    let bundle = runtime.product_surface(None).expect("product surface");
    let caller = ProductSurfaceCaller::new(
        TenantId::new("runtime-webui-tenant").unwrap(),
        UserId::new("runtime-webui-owner").unwrap(),
        Some(AgentId::new("runtime-webui-agent").unwrap()),
        None,
    );
    let created = invoke_product_command(
        bundle.as_ref(),
        caller.clone(),
        CREATE_THREAD_COMMAND,
        ProductCreateThreadRequest {
            client_action_id: Some("create-webui-stream-thread".to_string()),
            requested_thread_id: None,
            project_id: None,
        },
    )
    .await
    .expect("create webui thread");
    let submitted = invoke_product_command(
        bundle.as_ref(),
        caller.clone(),
        SUBMIT_TURN_COMMAND,
        ProductSubmitTurnRequest {
            client_action_id: Some("send-webui-stream-message".to_string()),
            thread_id: Some(created.thread.thread_id.to_string()),
            content: Some("hello webui stream".to_string()),
            attachments: Vec::new(),
            model: None,
        },
    )
    .await
    .expect("submit webui turn");
    let RebornSubmitTurnResponse::Submitted { run_id, .. } = submitted else {
        panic!("webui submit should start a run");
    };
    let stream = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let stream = stream_product_events(
                bundle.as_ref(),
                caller.clone(),
                RebornStreamEventsRequest {
                    thread_id: created.thread.thread_id.to_string(),
                    after_cursor: None,
                },
            )
            .await
            .expect("webui event stream");
            if stream.events.iter().any(|event| {
                matches!(
                    event.payload(),
                    ProductOutboundPayload::ProjectionSnapshot { state }
                        | ProductOutboundPayload::ProjectionUpdate { state }
                        if state.items.iter().any(|item| matches!(
                            item,
                            ProductProjectionItem::RunStatus {
                                run_id: seen,
                                status,
                                ..
                            }
                                if *seen == run_id && status == "completed"
                        ))
                )
            }) {
                break stream;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("completed webui projection should appear");

    let _api = bundle.clone();
    assert!(Arc::ptr_eq(
        &runtime_turn_coordinator,
        &runtime.product_turn_coordinator()
    ));
    assert!(
        stream.events.iter().all(|event| matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(_)
                | ProductOutboundPayload::CapabilityDisplayPreview(_)
                | ProductOutboundPayload::ProjectionSnapshot { .. }
                | ProductOutboundPayload::ProjectionUpdate { .. }
        )),
        "product surface should expose only projection stream events"
    );
    assert_eq!(runtime.readiness().state, RebornReadinessState::DevOnly);

    runtime.shutdown().await.expect("runtime shutdown");
}

/// Caller-level regression for the production attachment-landing path:
/// drives `RebornRuntime::webui_workspace_filesystem()` — the exact method
/// `runtime.product_surface`/`build_openai_compat_route_mount` call — through
/// a real `ProjectScopedAttachmentLander`, then reads the landed bytes back
/// through the same `ProjectScopedAttachmentReader` production wires
/// `attachment_read_port` with. The C-ATTACH integration tests exercise the
/// shared `RebornRuntimeStores::read_write_workspace_filesystem` recipe via the
/// `local_dev_attachment_test_support_for_test` seam, but never call through
/// this `RebornRuntime` wrapper itself; this closes that gap so a future
/// regression in the wrapper (not just the shared recipe) fails a test
/// instead of only breaking WebUI/OpenAI-compatible attachment landing in
/// production.
#[tokio::test]
async fn webui_workspace_filesystem_lands_attachment_with_read_write_mount() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "attachment mount ok".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-attachment-mount-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-attachment-mount-tenant".to_string(),
        agent_id: "runtime-attachment-mount-agent".to_string(),
        source_binding_id: "runtime-attachment-mount-source".to_string(),
        reply_target_binding_id: "runtime-attachment-mount-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let read_write_filesystem = runtime
        .webui_workspace_filesystem()
        .expect("local-dev runtime composes a read-write webui workspace filesystem");
    // The read port reads the same durable bytes the lander writes; production's
    // `attachment_read_port` uses the read-only workspace view, but the read side
    // is byte-identical over the read-write view (the reader never writes), so
    // this test resolves the same authority a vision-capable model would.
    let read_port =
        crate::support::fs::ProjectScopedAttachmentReader::new(Arc::clone(&read_write_filesystem));
    let lander = crate::support::fs::ProjectScopedAttachmentLander::new(read_write_filesystem);

    let thread_scope = ThreadScope {
        tenant_id: TenantId::new("runtime-attachment-mount-tenant").unwrap(),
        agent_id: AgentId::new("runtime-attachment-mount-agent").unwrap(),
        project_id: None,
        owner_user_id: Some(UserId::new("runtime-attachment-mount-owner").unwrap()),
        mission_id: None,
    };
    let refs = ironclaw_product::InboundAttachmentLander::land(
        &lander,
        &thread_scope,
        "msg-attachment-mount",
        vec![ironclaw_attachments::InboundAttachment {
            id: "att-0".to_string(),
            mime_type: "image/png".to_string(),
            filename: Some("mount-check.png".to_string()),
            bytes: b"attachment-mount-bytes".to_vec(),
        }],
    )
    .await
    .expect("landing through the production webui workspace filesystem succeeds");
    let storage_key = refs[0]
        .storage_key
        .as_deref()
        .expect("landed attachment carries a storage_key");

    let read_back = ironclaw_loop_host::LoopAttachmentReadPort::read_attachment_bytes(
        &read_port,
        &thread_scope.to_resource_scope(),
        storage_key,
    )
    .await
    .expect("reading the landed attachment back through the read port succeeds");

    assert_eq!(read_back, b"attachment-mount-bytes".to_vec());

    runtime.shutdown().await.expect("runtime shutdown");
}

async fn query_webui_extension_setup(
    api: &dyn ironclaw_host_api::ProductSurface,
    caller: ProductSurfaceCaller,
    package_id: &str,
) -> RebornSetupExtensionResponse {
    let page = query_product_surface_page(
        api,
        caller,
        ironclaw_product::RebornViewQuery {
            view_id: ironclaw_product::EXTENSION_SETUP_VIEW.id.to_string(),
            params: serde_json::json!({ "package_id": package_id }),
            cursor: None,
        },
    )
    .await
    .expect("setup extension lifecycle projection");
    serde_json::from_value(page.payload).expect("setup extension payload")
}

async fn invoke_product_command<T, O>(
    api: &dyn ironclaw_host_api::ProductSurface,
    caller: ProductSurfaceCaller,
    command: ProductSurfaceCommandDescriptor<T, O>,
    input: T,
) -> Result<O, ProductSurfaceError>
where
    T: serde::Serialize,
    O: serde::de::DeserializeOwned,
{
    let input = serde_json::to_value(input).map_err(ProductSurfaceError::internal_from)?;
    let response = ironclaw_host_api::ProductSurface::invoke(
        api,
        caller,
        ironclaw_host_api::ProductSurfaceInvokeRequest {
            operation_id: command.capability_id()?,
            input,
            activity_id: ActivityId::new(),
        },
    )
    .await?;
    serde_json::from_value(response.output).map_err(ProductSurfaceError::internal_from)
}

async fn invoke_product_capability<T>(
    api: &dyn ironclaw_host_api::ProductSurface,
    caller: ProductSurfaceCaller,
    capability_id: &str,
    input: T,
) -> Result<Resolution, ProductSurfaceError>
where
    T: serde::Serialize,
{
    let input = serde_json::to_value(input).map_err(ProductSurfaceError::internal_from)?;
    let response = ironclaw_host_api::ProductSurface::invoke(
        api,
        caller,
        ironclaw_host_api::ProductSurfaceInvokeRequest {
            operation_id: CapabilityId::new(capability_id).expect("capability id"),
            input,
            activity_id: ActivityId::new(),
        },
    )
    .await?;
    serde_json::from_value(response.output).map_err(ProductSurfaceError::internal_from)
}

async fn query_product_surface_page(
    api: &dyn ironclaw_host_api::ProductSurface,
    caller: ProductSurfaceCaller,
    query: RebornViewQuery,
) -> Result<RebornViewPage, ProductSurfaceError> {
    let page = ironclaw_host_api::ProductSurface::query(
        api,
        caller,
        ironclaw_host_api::ProductSurfaceQueryRequest {
            view_id: query.view_id,
            input: query.params,
            cursor: query.cursor,
            limit: None,
        },
    )
    .await?;
    let payload = page
        .items
        .into_iter()
        .next()
        .ok_or_else(ProductSurfaceError::internal)?;
    Ok(RebornViewPage {
        payload,
        next_cursor: page.next_cursor,
    })
}

async fn stream_product_events(
    api: &dyn ironclaw_host_api::ProductSurface,
    caller: ProductSurfaceCaller,
    request: RebornStreamEventsRequest,
) -> Result<RebornStreamEventsResponse, ProductSurfaceError> {
    let response = ironclaw_host_api::ProductSurface::stream_events(
        api,
        caller,
        ironclaw_host_api::ProductSurfaceStreamRequest {
            stream_id: Some(request.thread_id),
            after_cursor: request
                .after_cursor
                .map(|cursor| cursor.as_str().to_string()),
        },
    )
    .await?;
    let events = response
        .events
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ProductSurfaceError::internal_from)?;
    Ok(RebornStreamEventsResponse { events })
}

async fn submit_webui_extension_setup(
    api: &dyn ironclaw_host_api::ProductSurface,
    caller: ProductSurfaceCaller,
    package_id: &str,
    request: ProductSetupExtensionRequest,
) -> RebornSetupExtensionResponse {
    let mut input = serde_json::to_value(request).expect("setup request serializes");
    input
        .as_object_mut()
        .expect("setup request serializes as object")
        .insert(
            "extension_id".to_string(),
            serde_json::Value::String(package_id.to_string()),
        );
    let resolution = invoke_product_capability(
        api,
        caller.clone(),
        ironclaw_product::EXTENSION_SETUP_SUBMIT_CAPABILITY_ID,
        input,
    )
    .await
    .expect("submit extension setup");
    match resolution {
        Resolution::Done(outcome) if outcome.verdict.is_success() => {}
        other => panic!("extension setup submit did not succeed: {other:?}"),
    }
    query_webui_extension_setup(api, caller, package_id).await
}

#[tokio::test]
async fn local_dev_webui_bundle_uses_local_lifecycle_facade_for_setup_extension() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "webui lifecycle ok".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-webui-lifecycle-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy())
        .with_vendor_oauth_client(
            ironclaw_auth::GOOGLE_PROVIDER_ID,
            crate::OAuthClientConfig::new(
                "runtime-webui-google-client.apps.googleusercontent.com",
                "http://127.0.0.1/oauth/callback/google",
                None,
            )
            .expect("valid test Google OAuth client config"),
        ),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-webui-lifecycle-tenant".to_string(),
        agent_id: "runtime-webui-lifecycle-agent".to_string(),
        source_binding_id: "runtime-webui-lifecycle-source".to_string(),
        reply_target_binding_id: "runtime-webui-lifecycle-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let bundle = runtime.product_surface(None).expect("product surface");
    let caller = ProductSurfaceCaller::new(
        TenantId::new("runtime-webui-lifecycle-tenant").unwrap(),
        UserId::new("runtime-webui-lifecycle-owner").unwrap(),
        Some(AgentId::new("runtime-webui-lifecycle-agent").unwrap()),
        None,
    );

    let github_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
        .expect("valid package ref");
    install_webui_extension(bundle.as_ref(), caller.clone(), github_ref.clone()).await;

    let setup = query_webui_extension_setup(bundle.as_ref(), caller.clone(), "github").await;

    assert_eq!(setup.package_ref.id.as_str(), "github");
    // #6520 3-state public lifecycle: installed with an unprovided credential
    // projects as setup-needed (no Configured/Installed wire states remain).
    assert_eq!(setup.phase, LifecyclePublicState::SetupNeeded);
    assert!(setup.blockers.is_empty());
    assert_eq!(setup.secrets.len(), 1);
    assert_eq!(setup.secrets[0].name, "github_runtime_token");
    assert_eq!(setup.secrets[0].provider, "github");
    assert!(!setup.secrets[0].optional);
    assert!(!setup.secrets[0].provided);
    assert!(matches!(
        setup.secrets[0].setup,
        RebornExtensionCredentialSetup::ManualToken
    ));
    let google_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "google-calendar")
        .expect("valid package ref");
    install_webui_extension(bundle.as_ref(), caller.clone(), google_ref).await;
    let google_setup =
        query_webui_extension_setup(bundle.as_ref(), caller.clone(), "google-calendar").await;
    assert_eq!(google_setup.secrets.len(), 1);
    let google_secret = &google_setup.secrets[0];
    assert_eq!(google_secret.provider, "google");
    assert!(!google_secret.provided);
    let RebornExtensionCredentialSetup::OAuth { scopes, .. } = &google_secret.setup else {
        panic!("Google setup secret should use OAuth")
    };
    assert_eq!(
        scopes
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        [
            GOOGLE_CALENDAR_EVENTS_SCOPE.to_string(),
            GOOGLE_CALENDAR_READONLY_SCOPE.to_string(),
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
    );
    let google_setup_json =
        serde_json::to_value(&google_setup.secrets[0]).expect("serialize setup secret");
    assert_eq!(google_setup_json["setup"]["kind"], "oauth");
    assert!(
        matches!(
            setup.payload.as_ref(),
            Some(LifecycleProductPayload::ExtensionList { extensions, count })
                if *count == 1
                    && extensions.len() == 1
                    && extensions[0].summary.package_ref.id.as_str() == "github"
                    && extensions[0].summary.credential_requirements.len() == 1
        ),
        "local product surface should use the local lifecycle facade package projection"
    );
    assert!(
        !setup.blockers.iter().any(|blocker| matches!(
            blocker,
            LifecycleReadinessBlocker::Runtime { ref_id: Some(ref_id) }
                if ref_id.as_str() == "reborn_lifecycle_facade_unwired"
        )),
        "local product surface must not fall back to the default unwired facade"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_webui_bundle_exposes_outbound_preferences_facade() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "webui outbound ok".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-webui-outbound-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-webui-outbound-tenant".to_string(),
        agent_id: "runtime-webui-outbound-agent".to_string(),
        source_binding_id: "runtime-webui-outbound-source".to_string(),
        reply_target_binding_id: "runtime-webui-outbound-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let bundle = runtime.product_surface(None).expect("product surface");
    let caller = ProductSurfaceCaller::new(
        TenantId::new("runtime-webui-outbound-tenant").unwrap(),
        UserId::new("runtime-webui-outbound-owner").unwrap(),
        Some(AgentId::new("runtime-webui-outbound-agent").unwrap()),
        None,
    );

    let cleared = invoke_product_capability(
        bundle.as_ref(),
        caller.clone(),
        ironclaw_product::OUTBOUND_PREFERENCES_SET_CAPABILITY_ID,
        serde_json::json!({}),
    )
    .await
    .expect("outbound preference clear uses composed facade");
    assert!(matches!(cleared, Resolution::Done(_)));
    let cleared_page = query_product_surface_page(
        bundle.as_ref(),
        caller.clone(),
        ironclaw_product::RebornViewQuery {
            view_id: ironclaw_product::OUTBOUND_PREFERENCES_VIEW.id.to_string(),
            params: serde_json::json!({}),
            cursor: None,
        },
    )
    .await
    .expect("outbound preference read-back uses composed view");
    let cleared_preferences: RebornOutboundPreferencesResponse =
        serde_json::from_value(cleared_page.payload).expect("outbound preferences payload");
    assert!(cleared_preferences.final_reply_target.is_none());

    let targets_page = query_product_surface_page(
        bundle.as_ref(),
        caller,
        ironclaw_product::RebornViewQuery {
            view_id: ironclaw_product::OUTBOUND_DELIVERY_TARGETS_VIEW
                .id
                .to_string(),
            params: serde_json::json!({}),
            cursor: None,
        },
    )
    .await
    .expect("outbound target listing uses composed facade");
    let targets: ironclaw_product::RebornOutboundDeliveryTargetListResponse =
        serde_json::from_value(targets_page.payload).expect("outbound targets payload");
    // #6520: the host-owned WebApp final-reply destination is always
    // registered (host_owned_outbound_delivery_target_registry), so a runtime
    // with zero active channels lists exactly that one target.
    assert_eq!(targets.targets.len(), 1, "targets: {:?}", targets.targets);
    assert_eq!(targets.targets[0].target.channel.as_str(), "web_app");

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_webui_bundle_invokes_skill_install_with_scoped_mounts() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "webui skill ok".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-webui-skill-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-webui-skill-tenant".to_string(),
        agent_id: "runtime-webui-skill-agent".to_string(),
        source_binding_id: "runtime-webui-skill-source".to_string(),
        reply_target_binding_id: "runtime-webui-skill-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let bundle = runtime.product_surface(None).expect("product surface");
    let caller = ProductSurfaceCaller::new(
        TenantId::new("runtime-webui-skill-tenant").unwrap(),
        UserId::new("runtime-webui-skill-owner").unwrap(),
        Some(AgentId::new("runtime-webui-skill-agent").unwrap()),
        None,
    );

    let installed = invoke_product_capability(
        bundle.as_ref(),
        caller.clone(),
        ironclaw_product::SKILL_INSTALL_CAPABILITY_ID,
        serde_json::json!({
            "name": "product-surface-skill",
            "content": "---\nname: product-surface-skill\n---\n# Product Surface\n"
        }),
    )
    .await
    .expect("skill install uses product capability path");
    match installed {
        Resolution::Done(outcome) if outcome.verdict.is_success() => {}
        other => panic!("skill install did not succeed: {other:?}"),
    }
    let skills_page = query_product_surface_page(
        bundle.as_ref(),
        caller,
        ironclaw_product::RebornViewQuery {
            view_id: ironclaw_product::SKILLS_VIEW.id.to_string(),
            params: serde_json::json!({}),
            cursor: None,
        },
    )
    .await
    .expect("skill list uses product view");
    let skills: RebornSkillListResponse =
        serde_json::from_value(skills_page.payload).expect("skills payload");
    assert!(
        skills
            .skills
            .iter()
            .any(|skill| skill.name == "product-surface-skill")
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn webui_route_rejects_list_automations_without_agent_binding() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ironclaw_webui::webui_v2::{
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2State, webui_v2_router,
    };
    use tower::ServiceExt;

    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "unused".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-webui-no-agent-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-webui-no-agent-tenant".to_string(),
        agent_id: "runtime-webui-no-agent-agent".to_string(),
        source_binding_id: "runtime-webui-no-agent-source".to_string(),
        reply_target_binding_id: "runtime-webui-no-agent-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let bundle = runtime.product_surface(None).expect("product surface");
    let caller_without_agent = ProductSurfaceCaller::new(
        TenantId::new("runtime-webui-no-agent-tenant").unwrap(),
        UserId::new("runtime-webui-no-agent-owner").unwrap(),
        None,
        None,
    );
    let router = webui_v2_router(WebUiV2State::new(
        bundle,
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(caller_without_agent));

    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/webchat/v2/automations")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("route response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn webui_operator_diagnostics_route_exposes_composed_readiness_evidence() {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use ironclaw_webui::webui_v2::{
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2Capabilities, WebUiV2State, webui_v2_router,
    };
    use tower::ServiceExt;

    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "unused".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-webui-diagnostics-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-webui-diagnostics-tenant".to_string(),
        agent_id: "runtime-webui-diagnostics-agent".to_string(),
        source_binding_id: "runtime-webui-diagnostics-source".to_string(),
        reply_target_binding_id: "runtime-webui-diagnostics-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let bundle = runtime.product_surface(None).expect("product surface");
    let caller = ProductSurfaceCaller::new(
        TenantId::new("runtime-webui-diagnostics-tenant").unwrap(),
        UserId::new("runtime-webui-diagnostics-owner").unwrap(),
        Some(AgentId::new("runtime-webui-diagnostics-agent").unwrap()),
        None,
    )
    .with_operator_config(true);
    let router = webui_v2_router(WebUiV2State::new(
        bundle,
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(caller))
    .layer(axum::Extension(WebUiV2Capabilities {
        operator_webui_config: true,
    }));

    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/webchat/v2/operator/diagnostics")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("route response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("diagnostics json");
    assert!(
        json["operator_status"]["checks"]
            .as_array()
            .expect("status checks")
            .iter()
            .any(|check| check["id"] == "readiness_composition_profile"
                && check["status"] == "blocked"
                && check["summary"]
                    .as_str()
                    .is_some_and(|summary| summary.contains("reason=dev-only-profile"))),
        "diagnostics route should expose readiness-derived status checks: {json}"
    );
    assert!(
        json["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|diagnostic| diagnostic["reason_code"]
                == "operator_doctor_readiness_composition_profile_blocked"
                && diagnostic["key"] == "readiness_composition_profile"),
        "diagnostics route should expose readiness-derived doctor diagnostics: {json}"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn runtime_product_surface_without_local_runtime_still_lists_automations_from_core_store() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "unused".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-webui-no-host-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-webui-no-host-tenant".to_string(),
        agent_id: "runtime-webui-no-host-agent".to_string(),
        source_binding_id: "runtime-webui-no-host-source".to_string(),
        reply_target_binding_id: "runtime-webui-no-host-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let bundle = runtime.product_surface(None).expect("product surface");
    let caller = ProductSurfaceCaller::new(
        TenantId::new("runtime-webui-no-host-tenant").unwrap(),
        UserId::new("runtime-webui-no-host-owner").unwrap(),
        Some(AgentId::new("runtime-webui-no-host-agent").unwrap()),
        None,
    );

    let response = query_product_surface_page(
        bundle.as_ref(),
        caller,
        ironclaw_product::RebornViewQuery {
            view_id: ironclaw_product::AUTOMATIONS_VIEW.id.to_string(),
            params: serde_json::to_value(ProductListAutomationsRequest::default())
                .expect("automation list params"),
            cursor: None,
        },
    )
    .await
    .expect("automation facade reads the core trigger repository");

    let automations: ironclaw_product::RebornListAutomationsResponse =
        serde_json::from_value(response.payload).expect("automations payload");
    assert!(automations.automations.is_empty());
    assert!(!automations.scheduler_enabled);
    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_webui_setup_extension_stores_and_rotates_runtime_credentials() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "webui lifecycle ok".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-webui-credential-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-webui-credential-tenant".to_string(),
        agent_id: "runtime-webui-credential-agent".to_string(),
        source_binding_id: "runtime-webui-credential-source".to_string(),
        reply_target_binding_id: "runtime-webui-credential-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let bundle = runtime.product_surface(None).expect("product surface");
    let caller = ProductSurfaceCaller::new(
        TenantId::new("runtime-webui-credential-tenant").unwrap(),
        UserId::new("runtime-webui-credential-owner").unwrap(),
        Some(AgentId::new("runtime-webui-credential-agent").unwrap()),
        None,
    );
    let first = submit_webui_extension_setup(
        bundle.as_ref(),
        caller.clone(),
        "github",
        ProductSetupExtensionRequest {
            client_action_id: None,
            action: Some("submit".to_string()),
            payload: Some(serde_json::json!({
                "secrets": {
                    "github_runtime_token": "ghp_first_token"
                },
                "fields": {}
            })),
        },
    )
    .await;
    assert_eq!(first.secrets.len(), 1);
    assert!(first.secrets[0].provided);
    let first_credential_ref = first.secrets[0]
        .credential_ref
        .clone()
        .expect("credential ref");

    let second = submit_webui_extension_setup(
        bundle.as_ref(),
        caller,
        "github",
        ProductSetupExtensionRequest {
            client_action_id: None,
            action: Some("submit".to_string()),
            payload: Some(serde_json::json!({
                "secrets": {
                    "github_runtime_token": "ghp_second_token"
                },
                "fields": {}
            })),
        },
    )
    .await;
    assert_eq!(second.secrets.len(), 1);
    assert!(second.secrets[0].provided);
    assert_eq!(
        second.secrets[0].credential_ref.as_deref(),
        Some(first_credential_ref.as_str()),
        "reconfigure should rotate the existing account instead of creating a duplicate"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_webui_bundle_routes_approval_gates_into_interaction_service() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "unused".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-webui-approval-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-webui-approval-tenant".to_string(),
        agent_id: "runtime-webui-approval-agent".to_string(),
        source_binding_id: "runtime-webui-approval-source".to_string(),
        reply_target_binding_id: "runtime-webui-approval-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let bundle = runtime.product_surface(None).expect("product surface");
    let caller = ProductSurfaceCaller::new(
        TenantId::new("runtime-webui-approval-tenant").unwrap(),
        UserId::new("runtime-webui-approval-owner").unwrap(),
        Some(AgentId::new("runtime-webui-approval-agent").unwrap()),
        None,
    );
    let created = invoke_product_command(
        bundle.as_ref(),
        caller.clone(),
        CREATE_THREAD_COMMAND,
        ProductCreateThreadRequest {
            client_action_id: Some("create-webui-approval-thread".to_string()),
            requested_thread_id: None,
            project_id: None,
        },
    )
    .await
    .expect("create thread");
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate");

    let err = invoke_product_command::<_, ironclaw_product::RebornResolveGateResponse>(
        bundle.as_ref(),
        caller,
        RESOLVE_GATE_COMMAND,
        ProductResolveGateRequest {
            client_action_id: Some("resolve-webui-approval-gate".to_string()),
            thread_id: Some(created.thread.thread_id.to_string()),
            run_id: Some(TurnRunId::new().to_string()),
            gate_ref: Some(gate_ref.as_str().to_string()),
            resolution: Some("approved".to_string()),
            always: None,
            credential_ref: None,
        },
    )
    .await
    .expect_err("missing approval gate should reach approval interaction service");

    assert_eq!(err.code, ProductSurfaceErrorCode::NotFound);
    assert_eq!(err.kind, ProductSurfaceErrorKind::NotFound);
    assert_eq!(err.status_code, 404);
    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_webui_bundle_routes_auth_gates_into_interaction_service() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "unused".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-webui-auth-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-webui-auth-tenant".to_string(),
        agent_id: "runtime-webui-auth-agent".to_string(),
        source_binding_id: "runtime-webui-auth-source".to_string(),
        reply_target_binding_id: "runtime-webui-auth-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let bundle = runtime.product_surface(None).expect("product surface");
    let caller = ProductSurfaceCaller::new(
        TenantId::new("runtime-webui-auth-tenant").unwrap(),
        UserId::new("runtime-webui-auth-owner").unwrap(),
        Some(AgentId::new("runtime-webui-auth-agent").unwrap()),
        None,
    );
    let created = invoke_product_command(
        bundle.as_ref(),
        caller.clone(),
        CREATE_THREAD_COMMAND,
        ProductCreateThreadRequest {
            client_action_id: Some("create-webui-auth-thread".to_string()),
            requested_thread_id: None,
            project_id: None,
        },
    )
    .await
    .expect("create thread");

    let err = invoke_product_command::<_, ironclaw_product::RebornResolveGateResponse>(
        bundle.as_ref(),
        caller,
        RESOLVE_GATE_COMMAND,
        ProductResolveGateRequest {
            client_action_id: Some("resolve-webui-auth-gate".to_string()),
            thread_id: Some(created.thread.thread_id.to_string()),
            run_id: Some(TurnRunId::new().to_string()),
            gate_ref: Some("gate:hook-auth-missing".to_string()),
            resolution: Some("declined".to_string()),
            always: None,
            credential_ref: None,
        },
    )
    .await
    .expect_err("missing auth gate should reach auth interaction service");

    assert_eq!(err.code, ProductSurfaceErrorCode::NotFound);
    assert_eq!(err.kind, ProductSurfaceErrorKind::BlockedAuthentication);
    assert_eq!(err.status_code, 404);
    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn local_dev_webui_bundle_records_selectable_filesystem_skill_context() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let webui_helper_dir = user_skill_dir(
        &storage_root,
        "runtime-webui-skill-tenant",
        "runtime-webui-skill-user",
        "webui-helper",
    );
    std::fs::create_dir_all(&webui_helper_dir).expect("user skill dir");
    std::fs::write(
        webui_helper_dir.join("SKILL.md"),
        skill_md(
            "webui-helper",
            "webui helper description",
            "WEBUI_HELPER_PROMPT_SENTINEL",
        ),
    )
    .expect("write user skill");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        reply: "webui skill context ok".to_string(),
        requests: Arc::clone(&requests),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input("runtime-webui-skill-owner", storage_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-webui-skill-tenant".to_string(),
        agent_id: "runtime-webui-skill-agent".to_string(),
        source_binding_id: "runtime-webui-skill-source".to_string(),
        reply_target_binding_id: "runtime-webui-skill-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let bundle = runtime.product_surface(None).expect("product surface");
    let webui_user_id = UserId::new("runtime-webui-skill-user").unwrap();
    let caller = ProductSurfaceCaller::new(
        TenantId::new("runtime-webui-skill-tenant").unwrap(),
        webui_user_id.clone(),
        Some(AgentId::new("runtime-webui-skill-agent").unwrap()),
        None,
    );
    let created = invoke_product_command(
        bundle.as_ref(),
        caller.clone(),
        CREATE_THREAD_COMMAND,
        ProductCreateThreadRequest {
            client_action_id: Some("create-webui-skill-thread".to_string()),
            requested_thread_id: None,
            project_id: None,
        },
    )
    .await
    .expect("create thread");
    let submitted = invoke_product_command(
        bundle.as_ref(),
        caller,
        SUBMIT_TURN_COMMAND,
        ProductSubmitTurnRequest {
            client_action_id: Some("send-webui-skill-message".to_string()),
            thread_id: Some(created.thread.thread_id.to_string()),
            content: Some("$webui-helper please help".to_string()),
            attachments: Vec::new(),
            model: None,
        },
    )
    .await
    .expect("submit turn");
    let RebornSubmitTurnResponse::Submitted {
        thread_id,
        accepted_message_ref,
        ..
    } = submitted
    else {
        panic!("webui submit should start a run");
    };
    let resolved_run_profile = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .expect("resolve run profile");
    let source = runtime
        .webui_skill_activation_source()
        .expect("webui skill activation source");
    let turn_scope = TurnScope::new_with_owner(
        TenantId::new("runtime-webui-skill-tenant").unwrap(),
        Some(AgentId::new("runtime-webui-skill-agent").unwrap()),
        None,
        thread_id.clone(),
        Some(webui_user_id.clone()),
    );
    let context = LoopRunContext::new(
        turn_scope,
        TurnId::new(),
        TurnRunId::new(),
        resolved_run_profile,
    )
    .with_accepted_message_ref(accepted_message_ref)
    .with_actor(TurnActor::new(webui_user_id));
    let selected = source
        .load_skill_context_candidates(&context)
        .await
        .expect("webui-recorded skill context should load");
    let combined_skill_context = selected
        .iter()
        .map(|candidate| candidate.loaded_skill_md().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    // Default `listing` injection: the explicitly-mentioned skill loads its
    // full body; the bundled system skills collapse into one additional
    // `available-skills` listing candidate (description-only).
    assert!(combined_skill_context.contains("webui helper description"));
    assert!(combined_skill_context.contains("WEBUI_HELPER_PROMPT_SENTINEL"));
    let listing = selected
        .iter()
        .filter_map(|candidate| candidate.discoverable_metadata())
        .find(|(name, _)| *name == "available-skills")
        .map(|(_, listing)| listing.to_string())
        .expect("available-skills listing candidate");
    assert!(
        !listing.contains("WEBUI_HELPER_PROMPT_SENTINEL"),
        "listing must not carry skill bodies"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

/// Multi-call model response with a mid-register surface change must not kill the run.
///
/// Scenario: the scripted gateway (a) registers tool call #1, (b) activates an extension
/// (deterministic surface-content change), (c) registers tool call #2, then returns both
/// candidates together.  Before the fix, register #2 rebuilt the inner port, wiping the
/// snapshot that candidate #1 referred to; the executor hit StaleSurface on the first
/// candidate and collapsed to a terminal HostUnavailable failure.  After the fix, both
/// candidates carry the same (prompt-stage) surface version and the run completes.
#[tokio::test]
async fn multi_tool_call_response_survives_surface_change_mid_register() {
    use ironclaw_product::{
        LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
        LifecycleProductSurfaceContext,
    };
    use std::sync::OnceLock;

    // Gateway state seeded after runtime build.
    struct LifecycleFacadeHandle {
        facade: crate::extension_host::lifecycle::LifecycleFacade,
    }

    impl std::fmt::Debug for LifecycleFacadeHandle {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("LifecycleFacadeHandle").finish()
        }
    }

    struct MultiToolCallGateway {
        calls: StdMutex<usize>,
        facade_slot: Arc<OnceLock<LifecycleFacadeHandle>>,
    }

    #[async_trait]
    impl HostManagedModelGateway for MultiToolCallGateway {
        async fn stream_model(
            &self,
            _request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidRequest,
                "expected capability-aware model path",
            ))
        }

        async fn stream_model_with_capabilities(
            &self,
            _request: HostManagedModelRequest,
            capabilities: Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            let call_index = {
                let mut calls = self.calls.lock().expect("multi-tool gateway lock poisoned");
                let idx = *calls;
                *calls += 1;
                idx
            };

            if call_index > 0 {
                // Second model call: capability results have been fed back — finish the run.
                return Ok(HostManagedModelResponse::assistant_reply(
                    "multi-tool surface-change ok",
                ));
            }

            // ── First model call ──────────────────────────────────────────────────
            // Trigger prompt-stage surface snapshot (establishes V1).
            capabilities
                .visible_capabilities(VisibleCapabilityRequest)
                .await
                .map_err(model_capability_error)?;

            // Find the builtin echo tool.
            let echo_id = ironclaw_host_api::CapabilityId::new("builtin.echo").expect("echo id");
            let echo_tool = capabilities
                .tool_definitions()
                .map_err(model_capability_error)?
                .into_iter()
                .find(|def| def.capability_id == echo_id)
                .expect("echo provider tool definition");

            // Register call #1 — candidate carries surface version V1.
            let mut call1 = ProviderToolCall {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                turn_id: Some("provider-turn-multi".to_string()),
                id: "call-multi-1".to_string(),
                name: echo_tool.name.clone(),
                arguments: serde_json::json!({"message": "hello from call 1"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            };
            let candidate1 = capabilities
                .register_provider_tool_call(RegisterProviderToolCallRequest::new(call1.clone()))
                .await
                .map_err(model_capability_error)?;

            // Activate the github extension — deterministic surface-content change.
            // Pre-fix: this rebuilds the inner port, wiping candidate1's snapshot.
            let facade_handle = self
                .facade_slot
                .get()
                .expect("lifecycle facade must be seeded before send_user_message");
            let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
                .expect("valid github ref");
            // Install for the exact run user. Operator role never creates a
            // tenant-wide membership; install auto-reconciles readiness
            // (#6520 — there is no separate Activate action).
            let ctx = LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
                tenant_id: TenantId::new("tenant-multi-tool-surface").expect("tenant id"),
                user_id: UserId::new("runtime-multi-tool-surface-owner").expect("user id"),
                agent_id: None,
                project_id: None,
            });
            facade_handle
                .facade
                .execute(
                    ctx,
                    LifecycleProductAction::ExtensionInstall { package_ref },
                )
                .await
                .expect("install github extension");

            // Register call #2 — after surface change.
            // Post-fix: reuses current port, so both candidates carry the same surface version.
            call1.id = "call-multi-2".to_string();
            call1.arguments = serde_json::json!({"message": "hello from call 2"});
            let candidate2 = capabilities
                .register_provider_tool_call(RegisterProviderToolCallRequest::new(call1))
                .await
                .map_err(model_capability_error)?;

            // Both candidates must carry the same surface version after the fix.
            // (We cannot assert this here without breaking the pre-fix path,
            //  so we rely on the run-completion assertion in the test body.)
            Ok(HostManagedModelResponse::capability_calls(
                vec![candidate1, candidate2],
                "",
            ))
        }
    }

    // ── Test body ──────────────────────────────────────────────────────────────
    let root = tempfile::tempdir().expect("tempdir");
    let facade_slot: Arc<OnceLock<LifecycleFacadeHandle>> = Arc::new(OnceLock::new());
    let gateway = Arc::new(MultiToolCallGateway {
        calls: StdMutex::new(0),
        facade_slot: Arc::clone(&facade_slot),
    });
    let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway;

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-multi-tool-surface-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-multi-tool-surface-tenant".to_string(),
        agent_id: "runtime-multi-tool-surface-agent".to_string(),
        source_binding_id: "runtime-multi-tool-surface-source".to_string(),
        reply_target_binding_id: "runtime-multi-tool-surface-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: RUNTIME_POLL_TIMEOUT,
    })
    .with_model_gateway_override(gateway_for_runtime);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");

    // Seed the lifecycle facade before the model gateway runs.
    let extension_management = runtime.extension_management.clone();
    let facade = crate::extension_host::lifecycle::LifecycleFacade::new(Arc::clone(
        &runtime.skill_management,
    ))
    .with_extension_management(extension_management)
    .with_runtime_credential_accounts(Arc::new(MultiToolConfiguredCredentials));
    facade_slot
        .set(LifecycleFacadeHandle { facade })
        .expect("facade slot should be empty before seeding");

    let conversation = runtime.new_conversation().await.expect("conversation");
    runtime
        .enable_global_auto_approve_for_test(&conversation)
        .await;
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(&conversation, "use echo tool twice"),
    )
    .await
    .expect("runtime send should finish within timeout")
    .expect("runtime send should succeed");

    assert_eq!(
        reply.status,
        TurnStatus::Completed,
        "multi-tool response with mid-register surface change must not produce terminal failure; status={:?} text={:?}",
        reply.status,
        reply.text,
    );
    assert_eq!(reply.text.as_deref(), Some("multi-tool surface-change ok"));

    runtime.shutdown().await.expect("runtime shutdown");
}

/// Regression guard: a message that arrives while the thread is busy is stored with
/// `RejectedBusy` status and must NOT be auto-resubmitted when the blocking run
/// reaches a terminal state.
///
/// Scenario:
///  A – submitted via `turn_coordinator.submit_turn`; worker is stopped so it stays
///      Queued and holds the active-lock.
///  B – submitted via `bundle.submit_turn` (WebUI path); thread is busy → stored
///      as `RejectedBusy`; response carries a non-empty `notice`.
///  Cancel A → B stays `RejectedBusy` (no auto-resubmission).
///  C – submitted after A is cancelled; thread is free → `Submitted`.
///
/// arch-note: lives in runtime.rs (adds ~200 lines to an already >3000-line file) because
/// it requires `build_reborn_runtime` + full turn-runner control that only the runtime test
/// harness provides; moving it would require duplicating that harness. Decomposition of
/// runtime.rs is tracked in plan #4471.
#[tokio::test]
async fn rejected_busy_message_not_auto_resubmitted_after_run_cancellation() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "busy-drain ok".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });
    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "runtime-rejected-busy-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-rejected-busy-tenant".to_string(),
        agent_id: "runtime-rejected-busy-agent".to_string(),
        source_binding_id: "runtime-rejected-busy-source".to_string(),
        reply_target_binding_id: "runtime-rejected-busy-reply".to_string(),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    // Stop the worker so run A stays Queued and holds the thread active-lock.
    stop_turn_runner_worker_for_manual_state_test(&runtime).await;

    let bundle = runtime.product_surface(None).expect("product surface");
    let caller = ProductSurfaceCaller::new(
        TenantId::new("runtime-rejected-busy-tenant").unwrap(),
        UserId::new("runtime-rejected-busy-owner").unwrap(),
        Some(AgentId::new("runtime-rejected-busy-agent").unwrap()),
        None,
    );

    // Create the thread via WebUI so the thread record exists.
    let created = invoke_product_command(
        bundle.as_ref(),
        caller.clone(),
        CREATE_THREAD_COMMAND,
        ProductCreateThreadRequest {
            client_action_id: Some("create-rejected-busy-thread".to_string()),
            requested_thread_id: None,
            project_id: None,
        },
    )
    .await
    .expect("create thread");
    let thread_id = created.thread.thread_id.clone();

    // Submit message A directly so we hold the active-lock (worker is stopped,
    // so the run stays Queued indefinitely).
    let scope = caller.turn_scope(thread_id.clone());
    let actor = caller.actor();
    let submitted_a = runtime
        .turn_coordinator
        .submit_turn(SubmitTurnRequest {
            requested_model: None,
            scope: scope.clone(),
            actor: actor.clone(),
            accepted_message_ref: AcceptedMessageRef::new("msg:rejected-busy-a").unwrap(),
            source_binding_ref: SourceBindingRef::new("source:rejected-busy-a").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:rejected-busy-a").unwrap(),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new("rejected-busy-a").unwrap(),
            received_at: Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
        })
        .await
        .expect("message A submitted");
    let SubmitTurnResponse::Accepted {
        run_id: run_id_a, ..
    } = submitted_a;

    // Submit message B through the WebUI path — thread is busy, must get RejectedBusy.
    let response_b = invoke_product_command(
        bundle.as_ref(),
        caller.clone(),
        SUBMIT_TURN_COMMAND,
        ProductSubmitTurnRequest {
            client_action_id: Some("send-rejected-busy-b".to_string()),
            thread_id: Some(thread_id.to_string()),
            content: Some("message B while thread is busy".to_string()),
            attachments: Vec::new(),
            model: None,
        },
    )
    .await
    .expect("message B submit should not error");

    let RebornSubmitTurnResponse::RejectedBusy {
        notice: notice_b,
        active_run_id: busy_run_id,
        ..
    } = response_b
    else {
        panic!("expected RejectedBusy for message B, got {response_b:?}");
    };
    assert_eq!(
        busy_run_id,
        Some(run_id_a),
        "RejectedBusy should report run A as the active run"
    );
    assert!(
        !notice_b.is_empty(),
        "RejectedBusy response must carry a non-empty notice"
    );

    // Verify message B is stored with RejectedBusy status.
    let history = runtime
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: runtime.thread_scope.clone(),
            thread_id: thread_id.clone(),
        })
        .await
        .expect("thread history after B");
    let rejected_messages: Vec<_> = history
        .messages
        .iter()
        .filter(|m| matches!(m.status, MessageStatus::RejectedBusy))
        .collect();
    assert_eq!(
        rejected_messages.len(),
        1,
        "exactly one message should be stored as RejectedBusy after thread-busy submit"
    );
    assert_eq!(
        rejected_messages[0].kind,
        MessageKind::User,
        "the RejectedBusy message must be of kind User"
    );

    // Cancel run A — this is the terminal event that (must NOT) auto-resubmit B.
    runtime
        .cancel_run(
            &scope,
            run_id_a,
            SanitizedCancelReason::UserRequested,
            "rejected-busy-cancel-a",
        )
        .await
        .expect("run A cancellation succeeds");

    // B must remain RejectedBusy — no auto-resubmission should have fired.
    let history_after_cancel = runtime
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: runtime.thread_scope.clone(),
            thread_id: thread_id.clone(),
        })
        .await
        .expect("thread history after cancel");
    // Identify message B by the message_id we captured from the pre-cancel history.
    // Using the stable message_id (rather than a simple RejectedBusy count) ensures
    // a regression that leaves the RejectedBusy row AND adds a Submitted row for the
    // same message cannot slip past as "still one RejectedBusy".
    let msg_b_id = rejected_messages[0].message_id;

    let msg_b_after_cancel: Vec<_> = history_after_cancel
        .messages
        .iter()
        .filter(|m| m.message_id == msg_b_id)
        .collect();
    assert_eq!(
        msg_b_after_cancel.len(),
        1,
        "message B must appear exactly once in history after run A is cancelled"
    );
    assert_eq!(
        msg_b_after_cancel[0].status,
        MessageStatus::RejectedBusy,
        "message B must still be RejectedBusy after run A is cancelled — no auto-resubmission"
    );
    // Guard: no additional Submitted row must have been created for message B's message_id.
    let submitted_for_b: Vec<_> = history_after_cancel
        .messages
        .iter()
        .filter(|m| matches!(m.status, MessageStatus::Submitted) && m.message_id == msg_b_id)
        .collect();
    assert!(
        submitted_for_b.is_empty(),
        "no Submitted row must exist for message B after run A is cancelled — got {submitted_for_b:?}"
    );

    // Submit message C — thread is free again, must be Submitted.
    let response_c = invoke_product_command(
        bundle.as_ref(),
        caller.clone(),
        SUBMIT_TURN_COMMAND,
        ProductSubmitTurnRequest {
            client_action_id: Some("send-rejected-busy-c".to_string()),
            thread_id: Some(thread_id.to_string()),
            content: Some("message C after thread is free".to_string()),
            attachments: Vec::new(),
            model: None,
        },
    )
    .await
    .expect("message C submit should not error");

    assert!(
        matches!(response_c, RebornSubmitTurnResponse::Submitted { .. }),
        "message C must be accepted after run A is cancelled, got {response_c:?}"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

struct MultiToolConfiguredCredentials;

#[async_trait]
impl crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionService
    for MultiToolConfiguredCredentials
{
    async fn select_configured_account_for_binding(
        &self,
        _lookup: ironclaw_auth::CredentialAccountSelectionRequest,
        _runtime_scope: ironclaw_auth::AuthProductScope,
    ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
        Err(ironclaw_auth::AuthProductError::CredentialMissing)
    }

    async fn select_unique_configured_runtime_account(
        &self,
        _request: crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionRequest,
    ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
        let now = chrono::Utc::now();
        Ok(ironclaw_auth::CredentialAccount {
            id: ironclaw_auth::CredentialAccountId::new(),
            scope: ironclaw_auth::AuthProductScope::new(
                ironclaw_host_api::ResourceScope::local_default(
                    UserId::new("multi-tool-credential-user").expect("user id"),
                    ironclaw_host_api::InvocationId::new(),
                )
                .expect("resource scope"),
                ironclaw_auth::AuthSurface::Api,
            ),
            provider: ironclaw_auth::AuthProviderId::new("test-provider").expect("provider id"),
            label: ironclaw_auth::CredentialAccountLabel::new("test-provider")
                .expect("account label"),
            status: ironclaw_auth::CredentialAccountStatus::Configured,
            ownership: ironclaw_auth::CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(
                ironclaw_host_api::SecretHandle::new("test-secret").expect("secret handle"),
            ),
            refresh_secret: None,
            scopes: Vec::new(),
            provider_identity: None,
            created_at: now,
            updated_at: now,
        })
    }
}

// ── Regression: scheduler liveness must not treat mutex contention as stopped ──

/// Verify three invariants of the scheduler liveness check introduced to fix the
/// `try_lock()` contention bug:
///
/// 1. Before shutdown: liveness check says NOT stopped (atomic flag = false).
/// 2. While mutex is momentarily held by another task: atomic flag is still false,
///    so the guard correctly treats that as "alive".
/// 3. After graceful `shutdown()`: liveness check says stopped (atomic flag = true).
///
/// The `stopped` atomic flag is the authoritative signal; `try_lock`
/// failure now means "alive" rather than "stopped".
#[tokio::test]
async fn scheduler_liveness_not_stopped_under_contention() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "liveness-test-reply".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "scheduler-liveness-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "scheduler-liveness-tenant".to_string(),
        agent_id: "scheduler-liveness-agent".to_string(),
        source_binding_id: "scheduler-liveness-source".to_string(),
        reply_target_binding_id: "scheduler-liveness-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: RUNTIME_SEND_TIMEOUT,
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input)
        .await
        .expect("runtime builds for liveness test");

    let conversation = runtime.new_conversation().await.expect("conversation");

    // Invariant 1: Before shutdown, the atomic stopped flag must be false.
    assert!(
        !runtime.turn_scheduler.atomic_stopped(),
        "scheduler_stopped must be false on a freshly built runtime"
    );

    // Invariant 2: While the scheduler handle mutex is held (simulating
    // shutdown/scheduler contention), the public submit path must NOT
    // return `WorkerStopped` — and must complete within a bounded timeout.
    //
    // `is_stopped()` uses `try_lock()` (non-blocking) on the handle mutex,
    // not `lock().await`, so holding the lock here cannot deadlock. Tokio's
    // Mutex is non-re-entrant: `try_lock()` inside `is_stopped()` will
    // fail (returning `Err`) because the current task already holds the guard.
    // The guard falls through to "alive" because the `stopped` flag is false.
    //
    // `notify()` sends through the notifier (not the handle mutex), so the
    // worker processes the turn while the test holds the handle. The
    // RecordingGateway resolves the model call synchronously, so the turn
    // reaches Completed. We assert the full Ok result to catch both the
    // liveness regression (WorkerStopped) and any other scheduler breakage.
    //
    // The surrounding `tokio::time::timeout` is the deadlock-regression
    // guard: if `is_stopped()` ever regresses from `try_lock()` to
    // `lock().await`, this test will panic with a clear message instead of
    // hanging CI indefinitely.
    {
        // Hold the tokio Mutex for the duration of the submit call.
        let _guard = runtime.turn_scheduler.handle_mutex().lock().await;

        let result = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "liveness-probe"),
        )
        .await
        .expect(
            "send_user_message timed out while handle mutex was held — \
                 liveness guard likely regressed from try_lock() to lock().await, \
                 causing a deadlock",
        );

        assert!(
            result.is_ok(),
            "send_user_message must succeed (RecordingGateway completes the turn) \
                 while scheduler handle is merely contended (stopped=false); \
                 got: {result:?}"
        );
    } // guard released here — handle mutex is free again

    // Invariant 3: After the worker is stopped (flag = true), the public
    // submit path MUST return `WorkerStopped`.
    //
    // We use `stop_turn_runner_worker_for_manual_state_test` instead of
    // `shutdown()` because `shutdown()` consumes `self`, which would prevent
    // us from calling `send_user_message` afterward to exercise the guard.
    stop_turn_runner_worker_for_manual_state_test(&runtime).await;

    assert!(
        runtime.turn_scheduler.atomic_stopped(),
        "scheduler_stopped must be true after stop helper"
    );

    let result_after_stop = runtime
        .send_user_message(&conversation, "post-stop-probe")
        .await;
    assert!(
        matches!(
            result_after_stop,
            Err(super::RebornRuntimeError::WorkerStopped)
        ),
        "send_user_message must return WorkerStopped after scheduler is stopped; \
             got: {result_after_stop:?}"
    );

    // shutdown() handles the already-taken scheduler handle gracefully.
    runtime.shutdown().await.expect("runtime shutdown");
}

/// Companion test: `stop_turn_runner_worker_for_manual_state_test` (the test-only
/// helper used by many existing tests) must also set `scheduler_stopped = true`
/// so the liveness guard correctly reports stopped after it is called.
#[tokio::test]
async fn scheduler_liveness_stopped_after_test_helper_stops_worker() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "liveness-helper-test-reply".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "scheduler-liveness-helper-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "scheduler-liveness-helper-tenant".to_string(),
        agent_id: "scheduler-liveness-helper-agent".to_string(),
        source_binding_id: "scheduler-liveness-helper-source".to_string(),
        reply_target_binding_id: "scheduler-liveness-helper-reply".to_string(),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input)
        .await
        .expect("runtime builds for helper-stopped test");

    // Before stopping: not stopped.
    assert!(
        !runtime.turn_scheduler.atomic_stopped(),
        "scheduler_stopped must be false before stop helper runs"
    );

    stop_turn_runner_worker_for_manual_state_test(&runtime).await;

    // After the test helper stops the worker: flag must be true.
    assert!(
        runtime.turn_scheduler.atomic_stopped(),
        "scheduler_stopped must be true after stop_turn_runner_worker_for_manual_state_test"
    );

    // shutdown() handles the already-taken scheduler handle gracefully
    // via the `if let Some` guard — safe to call after the test helper.
    runtime.shutdown().await.expect("runtime shutdown");
}

/// After `stop_turn_runner_worker_for_manual_state_test` sets
/// `scheduler_stopped = true`, `send_user_message` must immediately return
/// `Err(RebornRuntimeError::WorkerStopped)` without submitting the turn.
#[tokio::test]
async fn scheduler_stopped_rejects_send_user_message() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(RecordingGateway {
        reply: "stopped-reject-reply".to_string(),
        requests: Arc::new(StdMutex::new(Vec::new())),
    });

    let input = RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "scheduler-stopped-reject-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "scheduler-stopped-reject-tenant".to_string(),
        agent_id: "scheduler-stopped-reject-agent".to_string(),
        source_binding_id: "scheduler-stopped-reject-source".to_string(),
        reply_target_binding_id: "scheduler-stopped-reject-reply".to_string(),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input)
        .await
        .expect("runtime builds for stopped-reject test");

    let conversation = runtime.new_conversation().await.expect("conversation");

    // Capture thread history before the stopped-send to verify no side effects.
    let thread_service = runtime.session_thread_service();
    let thread_scope = runtime.thread_scope.clone();
    let history_before = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope.clone(),
            thread_id: conversation.0.clone(),
        })
        .await
        .expect("list history before stopped send");

    stop_turn_runner_worker_for_manual_state_test(&runtime).await;

    let result = runtime.send_user_message(&conversation, "hi").await;
    assert!(
        matches!(result, Err(RebornRuntimeError::WorkerStopped)),
        "send_user_message must return WorkerStopped when scheduler is stopped, got: {result:?}"
    );

    // Assert no side effects: history must not grow after the rejected send.
    let history_after = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope,
            thread_id: conversation.0.clone(),
        })
        .await
        .expect("list history after stopped send");
    assert_eq!(
        history_before.messages.len(),
        history_after.messages.len(),
        "send_user_message must not write any messages when WorkerStopped is returned"
    );

    // shutdown() handles the already-taken scheduler handle gracefully.
    runtime.shutdown().await.expect("runtime shutdown");
}
// arch-exempt: large_file, runtime composition contract coverage remains centralized, plan #6175
