#[cfg(feature = "libsql")]
use std::collections::BTreeMap;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use std::sync::Arc;

#[cfg(feature = "libsql")]
use chrono::Utc;
#[cfg(feature = "postgres")]
use deadpool_postgres::tokio_postgres;
#[cfg(feature = "libsql")]
use ironclaw_auth::{OAuthClientId, OAuthRedirectUri};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::{
    AuditMode, DeploymentMode, EffectKind, FilesystemBackendKind, NetworkMode, PackageId,
    ProcessBackendKind, RuntimeKind, RuntimeProfile, SecretMode,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy},
};
#[cfg(feature = "libsql")]
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, ExecutionContext, ExtensionId,
    GrantConstraints, MountView, NetworkPolicy, Principal, TrustClass, UserId,
};
#[cfg(feature = "libsql")]
use ironclaw_host_runtime::{CapabilitySurfacePolicy, SurfaceKind, VisibleCapabilityRequest};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_runtime::{
    SchedulerTurnRunWakeNotifier, TurnRunExecutor, TurnRunExecutorError, TurnRunScheduler,
    TurnRunSchedulerConfig, TurnRunSchedulerHandle,
};
#[cfg(feature = "libsql")]
use ironclaw_reborn_composition::RebornRuntimeProcessBinding;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_reborn_composition::{RebornBuildError, RebornCompositionProfile};
use ironclaw_reborn_composition::{
    RebornBuildInput, RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest,
    RebornReadinessState, build_reborn_services,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_secrets::SecretMaterial;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
#[cfg(feature = "libsql")]
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_turns::{
    InMemoryTurnStateStore,
    runner::{ClaimedTurnRun, TurnRunTransitionPort},
};
use secrecy::SecretString;
#[cfg(feature = "libsql")]
use tokio::sync::Mutex;

#[cfg(feature = "libsql")]
static SECRETS_MASTER_KEY_ENV_LOCK: Mutex<()> = Mutex::const_new(());

#[cfg(feature = "libsql")]
struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
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

#[path = "facade_factory/sandbox_process_ports.rs"]
mod sandbox_process_ports;

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn test_master_key() -> SecretMaterial {
    SecretMaterial::from("01234567890123456789012345678901")
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
struct NoopTurnRunExecutor;

#[cfg(any(feature = "libsql", feature = "postgres"))]
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

#[cfg(any(feature = "libsql", feature = "postgres"))]
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

#[cfg(any(feature = "libsql", feature = "postgres"))]
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

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
fn empty_trust_policy() -> Arc<HostTrustPolicy> {
    Arc::new(HostTrustPolicy::empty())
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn live_wake_notifier() -> (Arc<SchedulerTurnRunWakeNotifier>, TurnRunSchedulerHandle) {
    let transitions: Arc<dyn TurnRunTransitionPort> = Arc::new(InMemoryTurnStateStore::default());
    let executor: Arc<dyn TurnRunExecutor> = Arc::new(NoopTurnRunExecutor);
    let handle =
        TurnRunScheduler::new(transitions, executor, TurnRunSchedulerConfig::default()).start();
    (handle.wake_notifier(), handle)
}

#[cfg(feature = "libsql")]
async fn libsql_db_at(path: impl AsRef<std::path::Path>) -> Arc<libsql::Database> {
    Arc::new(
        libsql::Builder::new_local(path.as_ref())
            .build()
            .await
            .unwrap(),
    )
}

#[cfg(feature = "postgres")]
async fn postgres_pool_or_skip() -> Option<(
    testcontainers_modules::testcontainers::ContainerAsync<
        testcontainers_modules::postgres::Postgres,
    >,
    deadpool_postgres::Pool,
    String,
)> {
    let (container, database_url) = start_postgres_container().await?;
    let config: tokio_postgres::Config = database_url
        .parse()
        .expect("testcontainer database URL must parse");
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("Postgres pool must build");
    let _connection = pool
        .get()
        .await
        .expect("Postgres testcontainer must accept connections");
    Some((container, pool, database_url))
}

#[cfg(feature = "postgres")]
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

#[cfg(feature = "libsql")]
fn test_sandbox_process_binding() -> RebornRuntimeProcessBinding {
    let process_port = Arc::new(ironclaw_host_runtime::TenantSandboxProcessPort::new(
        Arc::new(ProductionReadySandboxTransport),
    ));
    RebornRuntimeProcessBinding::tenant_sandbox(process_port)
}

#[cfg(feature = "libsql")]
#[derive(Debug)]
struct ProductionReadySandboxTransport;

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
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
        .with_google_oauth_backend(ironclaw_reborn_composition::OAuthClientConfig {
            client_id: OAuthClientId::new("google-client-123").unwrap(),
            client_secret: None,
            redirect_uri: OAuthRedirectUri::new("https://app.example/oauth/callback").unwrap(),
        })
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

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_requires_live_turn_wake_notifier() {
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

    assert!(matches!(
        result,
        Err(RebornBuildError::MissingTurnRunWakeNotifier)
    ));
}

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
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
}

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
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
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await;

    handle.shutdown().await;

    let services =
        result.expect("production libsql services should build with a sandbox process binding");
    assert_eq!(
        services.readiness.state,
        RebornReadinessState::ProductionValidated
    );
    assert!(services.host_runtime.is_some());
    assert!(services.turn_coordinator.is_some());
    assert!(services.product_auth.is_some());
}

#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "TODO(#3856): restore when tenant sandbox process-port wiring exists"]
async fn production_postgres_services_wire_first_party_runtime_http_egress() {
    // Restore the ProductionValidated readiness and host_runtime.health()
    // happy-path assertions that are temporarily fail-closed below.
}

#[cfg(feature = "libsql")]
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

#[cfg(feature = "postgres")]
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

#[cfg(feature = "libsql")]
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
    assert!(services.host_runtime.is_some());
    assert!(services.turn_coordinator.is_some());
}

#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "TODO(#3856): restore when tenant sandbox process-port wiring exists"]
async fn migration_dry_run_validates_postgres_planned_turn_profile() {
    // Restore the MigrationDryRunValidated readiness and planned-profile
    // submit_turn assertions that are temporarily fail-closed below.
}

#[cfg(feature = "libsql")]
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

#[cfg(feature = "postgres")]
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
