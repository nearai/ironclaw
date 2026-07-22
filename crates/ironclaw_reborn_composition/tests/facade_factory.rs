// arch-exempt: large_file, pre-existing ~1.9K-line facade test suite; this change is a net-zero rename of build_local_dev_secret_store_for_test call sites with no cases added, plan #6168
//
// Decomposition of this suite travels with the composition god-crate shrink
// (#6168); do not add unrelated cases here.
#[path = "support/postgres.rs"]
mod postgres_support;

use std::{collections::BTreeMap, sync::Arc};

use chrono::Utc;
use deadpool_postgres::tokio_postgres;
use ironclaw_auth::{OAuthClientId, OAuthRedirectUri};
use ironclaw_host_api::{
    AuditMode, DeploymentMode, EffectKind, FilesystemBackendKind, NetworkMode, PackageId,
    ProcessBackendKind, RuntimeKind, RuntimeProfile, SecretMode,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy},
};
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, ExecutionContext, ExtensionId,
    GrantConstraints, MountView, NetworkPolicy, Principal, ResourceEstimate, RunId, TrustClass,
    UserId,
};
use ironclaw_host_runtime::{
    CapabilitySurfacePolicy, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeFailureKind, SHELL_CAPABILITY_ID, SPAWN_SUBAGENT_CAPABILITY_ID, SurfaceKind,
    VisibleCapabilityRequest,
};
use ironclaw_reborn_composition::RebornRuntimeProcessBinding;
use ironclaw_reborn_composition::{RebornBuildError, RebornCompositionProfile, RebornServices};
use ironclaw_reborn_composition::{
    RebornBuildInput, RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest,
    RebornReadinessDiagnostic, RebornReadinessState, build_reborn_services,
};
use ironclaw_reborn_composition::{
    RebornReadinessDiagnosticComponent, RebornReadinessDiagnosticReason,
    RebornReadinessDiagnosticStatus,
};
use ironclaw_runner::turn_scheduler::{
    SchedulerTurnRunWakeNotifier, TurnRunExecutor, TurnRunExecutorError, TurnRunScheduler,
    TurnRunSchedulerConfig, TurnRunSchedulerHandle,
};
use ironclaw_secrets::SecretMaterial;
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use ironclaw_turns::{
    runner::{ClaimedTurnRun, TurnRunTransitionPort},
    test_support::in_memory_turn_state_store,
};
use postgres_support::assert_postgres_accepts_connections;
use secrecy::SecretString;
use serde_json::Value;
use serde_json::json;
use tokio::sync::Mutex;

static SECRETS_MASTER_KEY_ENV_LOCK: Mutex<()> = Mutex::const_new(());

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: tests serialize process-env mutation with
        // SECRETS_MASTER_KEY_ENV_LOCK and restore the prior value on drop.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: EnvVarGuard is only constructed while
        // SECRETS_MASTER_KEY_ENV_LOCK is held by this test module.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

fn test_master_key() -> SecretMaterial {
    SecretMaterial::from("01234567890123456789012345678901")
}

struct NoopTurnRunExecutor;

#[async_trait::async_trait]
impl TurnRunExecutor for NoopTurnRunExecutor {
    async fn execute_claimed_run(
        &self,
        _claimed: ClaimedTurnRun,
        _transitions: Arc<dyn TurnRunTransitionPort>,
    ) -> Result<(), TurnRunExecutorError> {
        Ok(())
    }
}

fn production_trust_policy() -> Arc<HostTrustPolicy> {
    Arc::new(
        HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries([
            AdminEntry::for_admin(
                PackageId::new("reborn-test").unwrap(),
                HostTrustAssignment::first_party(),
                vec![EffectKind::DispatchCapability],
                None,
            ),
        ]))])
        .unwrap(),
    )
}

fn production_runtime_policy() -> EffectiveRuntimePolicy {
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

fn hosted_secure_default_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::HostedMultiTenant,
        requested_profile: RuntimeProfile::SecureDefault,
        resolved_profile: RuntimeProfile::SecureDefault,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::None,
        network_mode: NetworkMode::Brokered,
        secret_mode: SecretMode::BrokeredHandles,
        approval_policy: ApprovalPolicy::AskAlways,
        audit_mode: AuditMode::Standard,
    }
}

fn local_only_runtime_policy() -> EffectiveRuntimePolicy {
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

fn local_only_minimal_approval_policy() -> EffectiveRuntimePolicy {
    let mut policy = local_only_runtime_policy();
    policy.requested_profile = RuntimeProfile::LocalYolo;
    policy.resolved_profile = RuntimeProfile::LocalYolo;
    policy.approval_policy = ApprovalPolicy::Minimal;
    policy
}

fn network_denied_runtime_policy() -> EffectiveRuntimePolicy {
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

fn local_dev_builtin_visible_request() -> VisibleCapabilityRequest {
    let grants = CapabilitySet {
        grants: vec![
            local_dev_grant("builtin.echo", vec![EffectKind::DispatchCapability]),
            local_dev_grant(
                "builtin.http",
                vec![EffectKind::DispatchCapability, EffectKind::Network],
            ),
            local_dev_grant(
                "builtin.http.save",
                vec![
                    EffectKind::DispatchCapability,
                    EffectKind::Network,
                    EffectKind::WriteFilesystem,
                ],
            ),
        ],
    };
    let context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap();

    let mut provider_trust = BTreeMap::new();
    provider_trust.insert(
        ExtensionId::new("builtin").unwrap(),
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: vec![
                    EffectKind::DispatchCapability,
                    EffectKind::Network,
                    EffectKind::WriteFilesystem,
                ],
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::AdminConfig,
            evaluated_at: Utc::now(),
        },
    );

    VisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap())
        .with_policy(CapabilitySurfacePolicy::allow_all())
        .with_provider_trust(provider_trust)
}

fn production_builtin_visible_request() -> VisibleCapabilityRequest {
    let context = production_process_capability_execution_context();

    VisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap())
        .with_policy(CapabilitySurfacePolicy::allow_all())
        .with_provider_trust(production_builtin_provider_trust())
}

fn production_process_capability_execution_context() -> ExecutionContext {
    let grants = CapabilitySet {
        grants: vec![
            local_dev_grant(
                SHELL_CAPABILITY_ID,
                vec![
                    EffectKind::DispatchCapability,
                    EffectKind::SpawnProcess,
                    EffectKind::ExecuteCode,
                    EffectKind::ReadFilesystem,
                    EffectKind::WriteFilesystem,
                    EffectKind::Network,
                ],
            ),
            local_dev_grant(
                SPAWN_SUBAGENT_CAPABILITY_ID,
                vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
            ),
        ],
    };
    ExecutionContext::local_default(
        UserId::new("production-user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap()
}

fn production_builtin_provider_trust() -> BTreeMap<ExtensionId, TrustDecision> {
    let mut provider_trust = BTreeMap::new();
    provider_trust.insert(
        ExtensionId::new("builtin").unwrap(),
        production_builtin_trust_decision(),
    );
    provider_trust
}

fn production_builtin_trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![
                EffectKind::DispatchCapability,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
            ],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::AdminConfig,
        evaluated_at: Utc::now(),
    }
}

fn assert_failed_capability(
    outcome: RuntimeCapabilityOutcome,
    capability_id: &str,
    expected_kind: RuntimeFailureKind,
    expected_message: &str,
) {
    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected failed {capability_id} invocation, got {outcome:?}");
    };
    assert_eq!(failure.capability_id.as_str(), capability_id);
    assert_eq!(failure.kind, expected_kind);
    let message = failure.message.as_deref().unwrap_or_default();
    assert!(
        message.contains(expected_message),
        "expected {capability_id} failure message to contain {expected_message:?}, got {:?}",
        failure.message
    );
    // Denial messages must explain the reason in plain language and never leak
    // internal planner enum tokens to the model (see #6386 and the
    // `builtin_http_runtime_policy_denial_stops_before_egress` sibling check).
    for token in ["ProcessBackendKind::", "NetworkMode::", "SecretMode::"] {
        assert!(
            !message.contains(token),
            "{capability_id} failure message leaked internal planner enum token {token:?}: {message}"
        );
    }
}

async fn assert_process_capabilities_unavailable_for_processless_runtime(
    services: &RebornServices,
    expected_shell_failure_kind: RuntimeFailureKind,
    expected_shell_failure_message: &str,
) {
    let runtime = services
        .host_runtime
        .as_deref()
        .expect("production services expose host runtime");
    let surface = runtime
        .visible_capabilities(production_builtin_visible_request())
        .await
        .expect("visible capabilities resolve");
    let ids = surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.as_str())
        .collect::<Vec<_>>();
    assert!(
        !ids.contains(&SHELL_CAPABILITY_ID),
        "builtin.shell must not be visible when process_backend == None: {ids:?}"
    );
    assert!(
        !ids.contains(&SPAWN_SUBAGENT_CAPABILITY_ID),
        "process-effect builtin.spawn_subagent must not be visible when process_backend == None: {ids:?}"
    );

    let shell_outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            production_process_capability_execution_context(),
            CapabilityId::new(SHELL_CAPABILITY_ID).unwrap(),
            ResourceEstimate::default(),
            json!({"command": "echo should-not-run"}),
        ))
        .await
        .expect("shell invocation returns an outcome");
    assert_failed_capability(
        shell_outcome,
        SHELL_CAPABILITY_ID,
        expected_shell_failure_kind,
        expected_shell_failure_message,
    );

    let spawn_outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            production_process_capability_execution_context(),
            CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
            ResourceEstimate::default(),
            json!({}),
        ))
        .await
        .expect("spawn_subagent invocation returns an outcome");
    assert_failed_capability(
        spawn_outcome,
        SPAWN_SUBAGENT_CAPABILITY_ID,
        RuntimeFailureKind::Authorization,
        "process execution is disabled",
    );
}

fn local_dev_grant(capability: &str, allowed_effects: Vec<EffectKind>) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: CapabilityId::new(capability).unwrap(),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects,
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

async fn invoke_trigger_management(
    runtime: &dyn ironclaw_host_runtime::HostRuntime,
    capability: &str,
    input: Value,
) -> Value {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            trigger_management_execution_context(),
            CapabilityId::new(capability).unwrap(),
            ResourceEstimate::default(),
            input,
        ))
        .await
        .expect("trigger management capability invoke");
    let RuntimeCapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed trigger management invocation, got {outcome:?}");
    };
    completed.output
}

fn trigger_management_execution_context() -> ExecutionContext {
    let grants = CapabilitySet {
        grants: vec![
            local_dev_grant(
                ironclaw_host_runtime::TRIGGER_CREATE_CAPABILITY_ID,
                vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
            ),
            local_dev_grant(
                ironclaw_host_runtime::TRIGGER_LIST_CAPABILITY_ID,
                vec![EffectKind::DispatchCapability],
            ),
            local_dev_grant(
                ironclaw_host_runtime::TRIGGER_REMOVE_CAPABILITY_ID,
                vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
            ),
        ],
    };
    let mut context = ExecutionContext::local_default(
        UserId::new("trigger-user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap();
    context.run_id = Some(RunId::new());
    context
}

fn empty_trust_policy() -> Arc<HostTrustPolicy> {
    Arc::new(HostTrustPolicy::empty())
}

fn live_wake_notifier() -> (Arc<SchedulerTurnRunWakeNotifier>, TurnRunSchedulerHandle) {
    let transitions: Arc<dyn TurnRunTransitionPort> = Arc::new(in_memory_turn_state_store());
    let executor: Arc<dyn TurnRunExecutor> = Arc::new(NoopTurnRunExecutor);
    let handle =
        TurnRunScheduler::new(transitions, executor, TurnRunSchedulerConfig::default()).start();
    (handle.wake_notifier(), handle)
}

async fn assert_production_services_ready_with_first_party_runtime(services: &RebornServices) {
    assert_eq!(
        services.readiness.state,
        RebornReadinessState::ProductionValidated
    );
    assert!(services.turn_coordinator.is_some());
    assert!(services.product_auth.is_some());

    let runtime = services
        .host_runtime
        .as_deref()
        .expect("production services expose host runtime");
    let health = runtime
        .health()
        .await
        .expect("production host runtime health should resolve");
    assert!(
        health.ready,
        "production host runtime should report first-party backend ready"
    );
    assert!(health.missing_runtime_backends.is_empty());
}

async fn libsql_db_at(path: impl AsRef<std::path::Path>) -> Arc<libsql::Database> {
    Arc::new(
        libsql::Builder::new_local(path.as_ref())
            .build()
            .await
            .unwrap(),
    )
}

async fn libsql_trigger_record_count(db: &libsql::Database) -> i64 {
    let conn = db.connect().expect("connect libsql db");
    let mut rows = conn
        .query("SELECT COUNT(*) FROM trigger_records", ())
        .await
        .expect("trigger table exists");
    let row = rows
        .next()
        .await
        .expect("read trigger table count row")
        .expect("trigger table count row");
    row.get(0).expect("trigger count")
}

async fn postgres_pool_or_skip() -> Option<(
    testcontainers_modules::testcontainers::ContainerAsync<
        testcontainers_modules::postgres::Postgres,
    >,
    deadpool_postgres::Pool,
    String,
)> {
    let (container, database_url) = start_postgres_container().await?;
    assert_postgres_accepts_connections(&database_url).await;
    let config: tokio_postgres::Config = database_url
        .parse()
        .expect("testcontainer database URL must parse");
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("Postgres pool must build");
    Some((container, pool, database_url))
}

async fn start_postgres_container() -> Option<(
    testcontainers_modules::testcontainers::ContainerAsync<
        testcontainers_modules::postgres::Postgres,
    >,
    String,
)> {
    use testcontainers_modules::testcontainers::{ImageExt, runners::AsyncRunner};

    let image = testcontainers_modules::postgres::Postgres::default()
        .with_db_name("ironclaw_test")
        .with_user("postgres")
        .with_password("postgres")
        .with_tag("16-alpine");

    let container = match image.start().await {
        Ok(container) => container,
        Err(error) => {
            eprintln!(
                "skipping Postgres composition tests: docker/testcontainers unavailable ({error})"
            );
            return None;
        }
    };
    let host = match container.get_host().await {
        Ok(host) => host,
        Err(error) => {
            eprintln!(
                "skipping Postgres composition tests: could not resolve container host ({error})"
            );
            return None;
        }
    };
    let port = match container.get_host_port_ipv4(5432).await {
        Ok(port) => port,
        Err(error) => {
            eprintln!(
                "skipping Postgres composition tests: could not resolve container port ({error})"
            );
            return None;
        }
    };
    Some((
        container,
        format!("postgres://postgres:postgres@{host}:{port}/ironclaw_test"),
    ))
}

#[tokio::test]
async fn disabled_returns_empty_services() {
    let services = build_reborn_services(RebornBuildInput::disabled("test-owner"))
        .await
        .unwrap();

    assert!(services.host_runtime.is_none());
    assert!(services.turn_coordinator.is_none());
    assert_eq!(services.readiness.state, RebornReadinessState::Disabled);
    assert_eq!(
        services.readiness.diagnostics,
        vec![RebornReadinessDiagnostic::disabled()]
    );
}

#[tokio::test]
async fn local_dev_builds_facades_without_production_claim() {
    let dir = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "test-owner",
        dir.path().to_path_buf(),
    ))
    .await
    .unwrap();

    assert!(services.host_runtime.is_some());
    assert!(services.turn_coordinator.is_some());
    assert_eq!(services.readiness.state, RebornReadinessState::DevOnly);
    assert!(services.readiness.facades.host_runtime);
    assert!(services.readiness.facades.turn_coordinator);
    assert!(services.readiness.facades.product_auth);
    assert!(services.product_auth.is_some());
}

#[tokio::test]
async fn hosted_single_tenant_volume_hides_process_capabilities() {
    let dir = tempfile::tempdir().unwrap();
    let input = ironclaw_reborn_composition::local_runtime_build_input_with_options(
        RebornCompositionProfile::HostedSingleTenantVolume,
        "hosted-volume-owner",
        dir.path().to_path_buf(),
        Default::default(),
    )
    .unwrap();
    let services = build_reborn_services(input).await.unwrap();

    assert_eq!(
        services.readiness.profile,
        RebornCompositionProfile::HostedSingleTenantVolume
    );
    assert_eq!(
        services.readiness.state,
        RebornReadinessState::HostedSingleTenantVolumePreviewValidated
    );
    assert_process_capabilities_unavailable_for_processless_runtime(
        &services,
        RuntimeFailureKind::Authorization,
        "process execution is disabled",
    )
    .await;
}

fn test_sandbox_process_binding() -> RebornRuntimeProcessBinding {
    let process_port = Arc::new(ironclaw_host_runtime::TenantSandboxProcessPort::new(
        Arc::new(ProductionReadySandboxTransport),
    ));
    RebornRuntimeProcessBinding::tenant_sandbox(process_port)
}

#[derive(Debug)]
struct ProductionReadySandboxTransport;

#[async_trait::async_trait]
impl ironclaw_host_runtime::SandboxCommandTransport for ProductionReadySandboxTransport {
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
            duration: std::time::Duration::ZERO,
        })
    }
}

#[tokio::test]
async fn local_dev_product_auth_entrypoint_redacts_manual_token_submit() {
    let dir = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "test-owner",
        dir.path().to_path_buf(),
    ))
    .await
    .unwrap();
    let product_auth = services
        .product_auth
        .as_ref()
        .expect("local-dev composes product auth");
    let scope = auth_scope("alice");
    let provider = ironclaw_auth::AuthProviderId::new("github").unwrap();
    let label = ironclaw_auth::CredentialAccountLabel::new("work github").unwrap();

    let challenge = product_auth
        .request_manual_token_setup(RebornManualTokenSetupRequest {
            scope: scope.clone(),
            provider: provider.clone(),
            label: label.clone(),
            continuation: ironclaw_auth::AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        })
        .await
        .unwrap();
    assert_eq!(challenge.provider, provider);
    assert_eq!(challenge.label, label);

    let submit = RebornManualTokenSubmitRequest::new(
        scope.clone(),
        challenge.interaction_id,
        SecretString::from("super-secret-token".to_string()),
    );
    let debug = format!("{submit:?}");
    assert!(!debug.contains("super-secret-token"));

    let result = product_auth.submit_manual_token(submit).await.unwrap();
    assert_eq!(
        result.status,
        ironclaw_auth::CredentialAccountStatus::Configured
    );

    let accounts = product_auth
        .credential_account_service()
        .list_accounts(ironclaw_auth::CredentialAccountListRequest::new(
            scope.clone(),
            provider,
        ))
        .await
        .unwrap();
    assert_eq!(accounts.accounts.len(), 1);
    let serialized = serde_json::to_string(&accounts).unwrap();
    assert!(!serialized.contains("super-secret-token"));
    assert!(!serialized.contains("manual-access-"));
}

fn auth_scope(user: &str) -> ironclaw_auth::AuthProductScope {
    ironclaw_auth::AuthProductScope::new(
        ironclaw_host_api::ResourceScope::local_default(
            ironclaw_host_api::UserId::new(user).unwrap(),
            ironclaw_host_api::InvocationId::new(),
        )
        .unwrap(),
        ironclaw_auth::AuthSurface::Web,
    )
    .with_session_id(ironclaw_auth::AuthSessionId::new(format!("session-{user}")).unwrap())
}

#[tokio::test]
async fn local_dev_runtime_policy_exposes_http_capability() {
    let dir = tempfile::tempdir().unwrap();
    let services = build_reborn_services(
        RebornBuildInput::local_dev("test-owner", dir.path().to_path_buf())
            .with_runtime_policy(local_only_runtime_policy()),
    )
    .await
    .unwrap();
    let runtime = services
        .host_runtime
        .expect("local dev exposes host runtime");

    let surface = runtime
        .visible_capabilities(local_dev_builtin_visible_request())
        .await
        .unwrap();
    let visible_ids = surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.as_str())
        .collect::<Vec<_>>();

    assert!(visible_ids.contains(&"builtin.echo"));
    assert!(
        visible_ids.contains(&"builtin.http"),
        "local-dev facade should expose host HTTP when the runtime policy allows network"
    );
    assert!(
        visible_ids.contains(&"builtin.http.save"),
        "local-dev facade should expose saved-body HTTP when network and filesystem are allowed"
    );
}

#[tokio::test]
async fn local_dev_runtime_policy_hides_http_capability() {
    let dir = tempfile::tempdir().unwrap();
    let services = build_reborn_services(
        RebornBuildInput::local_dev("test-owner", dir.path().to_path_buf())
            .with_runtime_policy(network_denied_runtime_policy()),
    )
    .await
    .unwrap();
    let runtime = services
        .host_runtime
        .expect("local dev exposes host runtime");

    let surface = runtime
        .visible_capabilities(local_dev_builtin_visible_request())
        .await
        .unwrap();
    let visible_ids = surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.as_str())
        .collect::<Vec<_>>();

    assert!(visible_ids.contains(&"builtin.echo"));
    assert!(
        !visible_ids.contains(&"builtin.http"),
        "local-dev facade must forward the supplied runtime policy before visible-surface filtering"
    );
    assert!(
        !visible_ids.contains(&"builtin.http.save"),
        "local-dev facade must hide saved-body HTTP when network is denied"
    );
}

#[tokio::test]
async fn production_requires_configured_trust_policy() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;

    let result = build_reborn_services(RebornBuildInput::libsql(
        RebornCompositionProfile::Production,
        "test-owner",
        db,
        dir.path().join("events.db").to_string_lossy(),
        None,
        test_master_key(),
    ))
    .await;

    assert!(matches!(
        result,
        Err(RebornBuildError::MissingProductionTrustPolicy)
    ));
}

#[tokio::test]
async fn production_google_oauth_config_uses_factory_built_product_auth_ports() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_vendor_oauth_client(
            "google",
            ironclaw_reborn_composition::OAuthClientConfig {
                client_id: OAuthClientId::new("google-client-123").unwrap(),
                client_secret: None,
                redirect_uri: OAuthRedirectUri::new("https://app.example/oauth/callback").unwrap(),
                hosted_domain_hint: None,
            },
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await;

    handle.shutdown().await;

    let services = result.expect("production Google OAuth should use durable product-auth ports");
    assert!(services.product_auth.is_some());
}

#[tokio::test]
async fn production_factory_built_product_auth_manual_token_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let services = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await
    .expect("production services should build durable product-auth ports");

    let product_auth = services
        .product_auth
        .as_ref()
        .expect("production composes product auth");
    let scope = auth_scope("alice");
    let provider = ironclaw_auth::AuthProviderId::new("manual-provider").unwrap();
    let label = ironclaw_auth::CredentialAccountLabel::new("manual production").unwrap();
    let challenge = product_auth
        .request_manual_token_setup(RebornManualTokenSetupRequest::new(
            scope.clone(),
            provider.clone(),
            label,
            ironclaw_auth::AuthContinuationRef::SetupOnly,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        ))
        .await
        .unwrap();

    let result = product_auth
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            scope.clone(),
            challenge.interaction_id,
            SecretString::from("production-manual-token"),
        ))
        .await
        .unwrap();
    assert_eq!(
        result.status,
        ironclaw_auth::CredentialAccountStatus::Configured
    );

    let accounts = product_auth
        .credential_account_service()
        .list_accounts(ironclaw_auth::CredentialAccountListRequest::new(
            scope, provider,
        ))
        .await
        .unwrap();
    assert_eq!(accounts.accounts.len(), 1);
    assert_eq!(accounts.accounts[0].id, result.account_id);

    handle.shutdown().await;
}

#[tokio::test]
async fn production_rejects_empty_trust_policy() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(empty_trust_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await;

    handle.shutdown().await;

    assert!(matches!(
        result,
        Err(RebornBuildError::EmptyProductionTrustPolicy)
    ));
}

#[tokio::test]
async fn production_self_mints_turn_wake_wiring() {
    // Production no longer requires an externally-supplied turn-run wake notifier:
    // `build_production_shaped` mints its own `SchedulerWakeWiring` so the
    // coordinator and scheduler always share one channel. A build with every other
    // required input present (and NO `.with_turn_run_wake_notifier`) must succeed.
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await;

    assert!(
        result.is_ok(),
        "production build must succeed with a self-minted wake wiring; got: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn production_requires_runtime_policy() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await;

    handle.shutdown().await;

    assert!(matches!(
        result,
        Err(RebornBuildError::MissingRuntimePolicy)
    ));
}

#[tokio::test]
async fn production_rejects_local_only_runtime_policy() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(local_only_runtime_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await;

    handle.shutdown().await;

    let Err(RebornBuildError::ProductionWiring { report }) = result else {
        panic!(
            "expected production wiring rejection for local-only runtime policy, got {result:?}"
        );
    };
    assert!(
        report.contains(
            ironclaw_host_runtime::ProductionWiringComponent::RuntimePolicy,
            ironclaw_host_runtime::ProductionWiringIssueKind::LocalOnlyImplementation,
        ),
        "local-only runtime policy should fail production wiring: {report:?}"
    );
    let diagnostics = RebornReadinessDiagnostic::from_production_wiring_report(
        RebornCompositionProfile::Production,
        &report,
    );
    assert!(
        RebornReadinessDiagnostic::from_production_wiring_report(
            RebornCompositionProfile::LocalDev,
            &report,
        )
        .is_empty(),
        "production wiring reports should not produce production diagnostics for local-dev profiles"
    );
    assert!(
        diagnostics.contains(
            &RebornReadinessDiagnostic::production_blocker(
                RebornCompositionProfile::Production,
                RebornReadinessDiagnosticComponent::RuntimePolicy,
                RebornReadinessDiagnosticReason::LocalOnly,
            )
            .expect("production profile should create a blocker")
        ),
        "runtime policy local-only issue should map to readiness diagnostics: {diagnostics:?}"
    );
    assert!(
        diagnostics.contains(
            &RebornReadinessDiagnostic::production_blocker(
                RebornCompositionProfile::Production,
                RebornReadinessDiagnosticComponent::RuntimeProcessPort,
                RebornReadinessDiagnosticReason::LocalOnly,
            )
            .expect("production profile should create a blocker")
        ),
        "runtime process port local-only issue should map to readiness diagnostics: {diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.status == RebornReadinessDiagnosticStatus::Blocking)
    );
    let serialized = serde_json::to_string(&diagnostics).unwrap();
    assert!(!serialized.contains("LocalOnlyImplementation"));
    assert!(!serialized.contains("EffectiveRuntimePolicy"));
    assert!(!serialized.contains("ironclaw_"));
    assert!(!serialized.contains("/root/"));
    assert!(!serialized.contains("postgres://"));
}

#[tokio::test]
async fn production_rejects_memory_libsql_event_store() {
    let db = Arc::new(
        libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap(),
    );
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            ":memory:",
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await;

    handle.shutdown().await;

    let error = result.expect_err("production must reject in-memory event store");
    let rendered = error.to_string();
    assert!(!rendered.contains("postgres://"));
    assert!(!rendered.contains("token"));
}

#[tokio::test]
async fn production_libsql_resolved_secret_master_key_rejects_invalid_env_key() {
    let _guard = SECRETS_MASTER_KEY_ENV_LOCK.lock().await;
    let _env = EnvVarGuard::set(
        ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV,
        "correct horse battery staple pad!!",
    );
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql_with_resolved_secret_master_key(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await;

    handle.shutdown().await;

    assert!(matches!(
        result,
        Err(RebornBuildError::Secret(
            ironclaw_secrets::SecretError::InvalidMasterKey
        ))
    ));
}

/// With no cached dotfile and no `SECRETS_MASTER_KEY` env var,
/// `resolve_local_dev_secret_master_key` (`src/factory.rs`) tries the OS
/// keychain before generating a fresh key.
///
/// - Under `IRONCLAW_DISABLE_OS_KEYCHAIN` the keychain lookup returns
///   `NotFound`, so the resolver must fall through to "generate + persist a
///   dotfile"; a second open over the same root must read that cached
///   dotfile rather than re-generating.
/// - Lives here, not as a `factory.rs` inline unit test: proving the
///   fallthrough needs the real process env var `IRONCLAW_DISABLE_OS_KEYCHAIN`
///   set (`keychain` reads raw `std::env`), and `set_var` is `unsafe` under
///   edition 2024 — `ironclaw_reborn_composition` is `#![forbid(unsafe_code)]`,
///   which even `#[cfg(test)]` can't locally downgrade. This `tests/*.rs`
///   binary is a separate crate the `forbid` doesn't reach, and already uses
///   the `EnvVarGuard`/`SECRETS_MASTER_KEY_ENV_LOCK` convention for this.
#[tokio::test]
async fn local_dev_secret_store_falls_through_suppressed_keychain_to_dotfile() {
    let _guard = SECRETS_MASTER_KEY_ENV_LOCK.lock().await;
    let _env = EnvVarGuard::set("IRONCLAW_DISABLE_OS_KEYCHAIN", "1");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let key_path = root.join(".reborn-local-dev-secrets-master-key");
    assert!(
        !key_path.exists(),
        "precondition: no cached dotfile before the first open"
    );

    let mut composite = ironclaw_filesystem::CompositeRootFilesystem::new();
    ironclaw_reborn_composition::test_support::build_default_local_dev_database_roots_for_test(
        root,
        &mut composite,
    )
    .await
    .expect("build default local-dev db roots");
    let composite = std::sync::Arc::new(composite);
    let scoped = ironclaw_reborn_composition::wrap_scoped(std::sync::Arc::clone(&composite));

    ironclaw_reborn_composition::test_support::build_secret_store_for_test(
        root,
        std::sync::Arc::clone(&scoped),
    )
    .await
    .expect("first store build must fall through the suppressed keychain to a dotfile");
    assert!(
        key_path.exists(),
        "the fallthrough must persist a dotfile so subsequent boots don't hit the keychain again"
    );
    let cached = std::fs::read_to_string(&key_path).expect("read generated dotfile");

    ironclaw_reborn_composition::test_support::build_secret_store_for_test(root, scoped)
        .await
        .expect("second store build must read the now-cached dotfile idempotently");
    assert_eq!(
        std::fs::read_to_string(&key_path).expect("read dotfile again"),
        cached,
        "the cached dotfile must not be rewritten on the idempotent second open"
    );
}

#[tokio::test]
async fn production_libsql_services_wire_first_party_runtime_http_egress() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier)
        .with_runtime_process_binding(test_sandbox_process_binding())
        .with_required_runtime_backends([RuntimeKind::FirstParty])
        .require_runtime_http_egress(),
    )
    .await;

    handle.shutdown().await;

    let services =
        result.expect("production libsql services should build with a sandbox process binding");
    assert_production_services_ready_with_first_party_runtime(&services).await;
}

#[tokio::test]
async fn production_libsql_services_migrate_trigger_repository_before_runtime_injection() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let services = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            Arc::clone(&db),
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await
    .expect("production libsql services should build with trigger repository migrations");

    handle.shutdown().await;

    assert!(services.host_runtime.is_some());

    let conn = db.connect().expect("connect libsql state db");
    let mut rows = conn
        .query("SELECT COUNT(*) FROM trigger_records", ())
        .await
        .expect("trigger table exists after production build");
    let row = rows
        .next()
        .await
        .expect("read trigger table count row")
        .expect("trigger table count row");
    let count: i64 = row.get(0).expect("trigger table count");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn local_dev_services_dispatch_trigger_management_through_composed_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let services = build_reborn_services(
        RebornBuildInput::local_dev("test-owner", dir.path().to_path_buf())
            .with_runtime_policy(local_only_minimal_approval_policy()),
    )
    .await
    .expect("local-dev services should build with trigger management runtime");

    // The Tools-settings global auto-approve switch is authoritative for
    // first-party tool dispatch; turn it on for the dispatch scope so
    // these trigger management calls exercise the dispatch path instead of
    // stopping at the per-tool approval gate.
    let auto_approve = services
        .local_dev_auto_approve_settings_for_test()
        .expect("local-dev exposes auto-approve settings for test");
    let auto_approve_scope = trigger_management_execution_context().resource_scope;
    auto_approve
        .set(ironclaw_approvals::AutoApproveSettingInput {
            updated_by: Principal::User(auto_approve_scope.user_id.clone()),
            scope: auto_approve_scope,
            enabled: true,
        })
        .await
        .expect("enable global auto-approve for trigger management dispatch");

    let runtime = services
        .host_runtime
        .as_deref()
        .expect("local-dev build exposes host runtime");
    let created = invoke_trigger_management(
        runtime,
        ironclaw_host_runtime::TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "Daily production summary",
            "prompt": "Summarize production state",
            "schedule": { "kind": "cron", "expression": "0 8 * * *", "timezone": "UTC" }
        }),
    )
    .await;
    let trigger_id = created["trigger"]["trigger_id"]
        .as_str()
        .expect("created trigger id")
        .to_string();

    let local_dev_db = libsql_db_at(dir.path().join("reborn-local-dev.db")).await;
    assert_eq!(libsql_trigger_record_count(&local_dev_db).await, 1);

    let listed = invoke_trigger_management(
        runtime,
        ironclaw_host_runtime::TRIGGER_LIST_CAPABILITY_ID,
        json!({}),
    )
    .await;
    assert_eq!(
        listed["triggers"].as_array().expect("trigger list").len(),
        1
    );

    let removed = invoke_trigger_management(
        runtime,
        ironclaw_host_runtime::TRIGGER_REMOVE_CAPABILITY_ID,
        json!({ "trigger_id": trigger_id }),
    )
    .await;
    assert_eq!(removed["removed"], json!(true));

    let listed_after_remove = invoke_trigger_management(
        runtime,
        ironclaw_host_runtime::TRIGGER_LIST_CAPABILITY_ID,
        json!({}),
    )
    .await;
    assert_eq!(
        listed_after_remove["triggers"]
            .as_array()
            .expect("trigger list after remove")
            .len(),
        0
    );
}

#[tokio::test]
async fn production_postgres_services_migrate_trigger_repository_before_runtime_injection() {
    let Some((_container, pool, database_url)) = postgres_pool_or_skip().await else {
        return;
    };
    let (notifier, handle) = live_wake_notifier();

    let services = build_reborn_services(
        RebornBuildInput::postgres(
            RebornCompositionProfile::Production,
            "test-owner",
            pool.clone(),
            SecretMaterial::from(database_url),
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await
    .expect("production postgres services should build with trigger repository migrations");

    handle.shutdown().await;

    assert!(services.host_runtime.is_some());

    let client = pool.get().await.expect("connect postgres state db");
    let row = client
        .query_one("SELECT COUNT(*) FROM trigger_records", &[])
        .await
        .expect("trigger table exists after production build");
    let count: i64 = row.get(0);
    assert_eq!(count, 0);
}

#[tokio::test]
async fn production_postgres_services_wire_first_party_runtime_http_egress() {
    let Some((_container, pool, database_url)) = postgres_pool_or_skip().await else {
        return;
    };
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::postgres(
            RebornCompositionProfile::Production,
            "test-owner",
            pool,
            SecretMaterial::from(database_url),
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier)
        .with_runtime_process_binding(test_sandbox_process_binding())
        .with_required_runtime_backends([RuntimeKind::FirstParty])
        .require_runtime_http_egress(),
    )
    .await;

    handle.shutdown().await;

    let services =
        result.expect("production postgres services should build with a sandbox process binding");
    assert_production_services_ready_with_first_party_runtime(&services).await;
}

#[tokio::test]
async fn production_postgres_secure_default_builds_without_process_port() {
    let Some((_container, pool, database_url)) = postgres_pool_or_skip().await else {
        return;
    };
    let (notifier, handle) = live_wake_notifier();

    let services = build_reborn_services(
        RebornBuildInput::postgres(
            RebornCompositionProfile::Production,
            "test-owner",
            pool,
            SecretMaterial::from(database_url),
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(hosted_secure_default_runtime_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await
    .expect("postgres secure_default production should not require a process port");

    handle.shutdown().await;

    assert_production_services_ready_with_first_party_runtime(&services).await;
    assert_process_capabilities_unavailable_for_processless_runtime(
        &services,
        RuntimeFailureKind::MissingRuntime,
        "unknown capability",
    )
    .await;
}

#[tokio::test]
async fn production_libsql_secure_default_builds_without_process_port() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let services = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(hosted_secure_default_runtime_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await
    .expect("secure_default production should not require a process port");

    handle.shutdown().await;

    assert_production_services_ready_with_first_party_runtime(&services).await;
    assert_process_capabilities_unavailable_for_processless_runtime(
        &services,
        RuntimeFailureKind::MissingRuntime,
        "unknown capability",
    )
    .await;
}

#[tokio::test]
async fn production_libsql_services_require_process_port_for_first_party_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("reborn.db");
    let db = libsql_db_at(&db_path).await;
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier)
        .with_required_runtime_backends([RuntimeKind::FirstParty])
        .require_runtime_http_egress(),
    )
    .await;

    handle.shutdown().await;

    let Err(RebornBuildError::InvalidConfig { reason }) = result else {
        panic!("expected production first-party runtime to require a process port, got {result:?}");
    };
    assert!(
        reason.contains("tenant sandbox process binding"),
        "first-party shell capability should keep production wiring fail-closed until a tenant sandbox process port is configured: {reason}"
    );
}

#[tokio::test]
async fn production_postgres_services_require_process_port_for_first_party_runtime() {
    let Some((_container, pool, database_url)) = postgres_pool_or_skip().await else {
        return;
    };
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::postgres(
            RebornCompositionProfile::Production,
            "test-owner",
            pool,
            SecretMaterial::from(database_url),
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier)
        .with_required_runtime_backends([RuntimeKind::FirstParty])
        .require_runtime_http_egress(),
    )
    .await;

    handle.shutdown().await;

    let Err(RebornBuildError::InvalidConfig { reason }) = result else {
        panic!(
            "expected postgres production first-party runtime to require a process port, got {result:?}"
        );
    };
    assert!(
        reason.contains("tenant sandbox process binding"),
        "postgres first-party shell capability should keep production wiring fail-closed until a tenant sandbox process port is configured: {reason}"
    );
}

#[tokio::test]
async fn migration_dry_run_validates_libsql_shape() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::MigrationDryRun,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await;

    handle.shutdown().await;

    let services = result
        .expect("migration dry-run libsql services should build with a sandbox process binding");
    assert_eq!(
        services.readiness.state,
        RebornReadinessState::MigrationDryRunValidated
    );
    assert!(services.readiness.diagnostics.is_empty());
    assert!(services.host_runtime.is_some());
    assert!(services.turn_coordinator.is_some());
}

#[tokio::test]
#[ignore = "TODO(#3856): restore when tenant sandbox process-port wiring exists"]
async fn migration_dry_run_validates_postgres_planned_turn_profile() {
    // Restore the MigrationDryRunValidated readiness and planned-profile
    // submit_turn assertions that are temporarily fail-closed below.
}

#[tokio::test]
async fn migration_dry_run_requires_libsql_process_port_for_first_party_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("reborn.db");
    let db = libsql_db_at(&db_path).await;
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::MigrationDryRun,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await;

    handle.shutdown().await;

    let Err(RebornBuildError::InvalidConfig { reason }) = result else {
        panic!("expected migration dry-run to require a process port, got {result:?}");
    };
    assert!(
        reason.contains("tenant sandbox process binding"),
        "migration dry-run should keep production-shaped first-party wiring fail-closed until a tenant sandbox process port is configured: {reason}"
    );
}

#[tokio::test]
async fn migration_dry_run_requires_postgres_process_port_for_first_party_runtime() {
    let Some((_container, pool, database_url)) = postgres_pool_or_skip().await else {
        return;
    };
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::postgres(
            RebornCompositionProfile::MigrationDryRun,
            "test-owner",
            pool,
            SecretMaterial::from(database_url),
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await;

    handle.shutdown().await;

    let Err(RebornBuildError::InvalidConfig { reason }) = result else {
        panic!("expected postgres migration dry-run to require a process port, got {result:?}");
    };
    assert!(
        reason.contains("tenant sandbox process binding"),
        "postgres migration dry-run should keep production-shaped first-party wiring fail-closed until a tenant sandbox process port is configured: {reason}"
    );
}
