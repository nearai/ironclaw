#![cfg(feature = "libsql")]

use std::sync::Arc;

use ironclaw_auth::{
    AuthContinuationRef, AuthProductScope, AuthProviderId, AuthSessionId, AuthSurface,
    CredentialAccountLabel, CredentialAccountLookupRequest,
};
use ironclaw_filesystem::LibSqlRootFilesystem;
use ironclaw_host_api::{
    AuditMode, DeploymentMode, EffectKind, FilesystemBackendKind, InvocationId, NetworkMode,
    PackageId, ProcessBackendKind, ResourceScope, RuntimeProfile, SecretHandle, SecretMode, UserId,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy},
};
use ironclaw_host_runtime::{
    SchedulerTurnRunWakeNotifier, TurnRunExecutor, TurnRunExecutorError, TurnRunScheduler,
    TurnRunSchedulerConfig, TurnRunSchedulerHandle,
};
use ironclaw_reborn_composition::{
    RebornBuildInput, RebornCompositionProfile, RebornManualTokenSetupRequest,
    RebornManualTokenSubmitRequest, RebornRuntimeProcessBinding, RebornServices,
    build_reborn_services,
};
use ironclaw_secrets::{
    FilesystemSecretStore, SecretMaterial, SecretStore, SecretStoreError, SecretsCrypto,
};
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
use ironclaw_turns::{
    InMemoryTurnStateStore,
    runner::{ClaimedTurnRun, TurnRunTransitionPort},
};
use secrecy::{ExposeSecret, SecretString};

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

fn live_wake_notifier() -> (Arc<SchedulerTurnRunWakeNotifier>, TurnRunSchedulerHandle) {
    let transitions: Arc<dyn TurnRunTransitionPort> = Arc::new(InMemoryTurnStateStore::default());
    let executor: Arc<dyn TurnRunExecutor> = Arc::new(NoopTurnRunExecutor);
    let handle =
        TurnRunScheduler::new(transitions, executor, TurnRunSchedulerConfig::default()).start();
    (handle.wake_notifier(), handle)
}

async fn libsql_db_at(path: impl AsRef<std::path::Path>) -> Arc<libsql::Database> {
    Arc::new(
        libsql::Builder::new_local(path.as_ref())
            .build()
            .await
            .unwrap(),
    )
}

async fn build_production_services_for_db(
    db: Arc<libsql::Database>,
    events_path: impl Into<String>,
    notifier: Arc<SchedulerTurnRunWakeNotifier>,
) -> RebornServices {
    build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            events_path.into(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await
    .expect("production services should build durable product-auth ports")
}

async fn production_secret_store_for_db(
    db: Arc<libsql::Database>,
) -> FilesystemSecretStore<LibSqlRootFilesystem> {
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db));
    filesystem.run_migrations().await.unwrap();
    FilesystemSecretStore::new(
        ironclaw_reborn_composition::wrap_scoped(filesystem),
        Arc::new(SecretsCrypto::new(test_master_key()).unwrap()),
    )
}

async fn consume_secret(
    store: &dyn SecretStore,
    scope: &ResourceScope,
    handle: &SecretHandle,
) -> String {
    let lease = store
        .lease_once(scope, handle)
        .await
        .expect("secret lease should be created");
    store
        .consume(scope, lease.id)
        .await
        .expect("secret lease should expose material")
        .expose_secret()
        .to_string()
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

fn auth_scope(user: &str) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope::local_default(UserId::new(user).unwrap(), InvocationId::new()).unwrap(),
        AuthSurface::Web,
    )
    .with_session_id(AuthSessionId::new(format!("session-{user}")).unwrap())
}

#[tokio::test]
async fn production_manual_token_secret_material_survives_service_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let services = build_production_services_for_db(
        Arc::clone(&db),
        dir.path().join("events.db").to_string_lossy(),
        notifier,
    )
    .await;

    let product_auth = services
        .product_auth
        .as_ref()
        .expect("production composes product auth");
    let scope = auth_scope("alice");
    let provider = AuthProviderId::new("manual-provider").unwrap();
    let label = CredentialAccountLabel::new("durable manual").unwrap();
    let challenge = product_auth
        .request_manual_token_setup(RebornManualTokenSetupRequest::new(
            scope.clone(),
            provider.clone(),
            label,
            AuthContinuationRef::SetupOnly,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        ))
        .await
        .expect("manual-token setup should create challenge");

    let submitted = product_auth
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            scope.clone(),
            challenge.interaction_id,
            SecretString::from("production-secret-before-rebuild"),
        ))
        .await
        .expect("manual-token submit should persist secret");
    let account = product_auth
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            submitted.account_id,
        ))
        .await
        .expect("account lookup should succeed")
        .expect("manual-token submit should create an account");
    let access_secret = account
        .access_secret
        .clone()
        .expect("manual-token account should reference a secret handle");
    assert!(
        access_secret.as_str().starts_with("product-auth-manual-"),
        "manual-token account should reference a product-auth SecretStore handle"
    );

    let secret_store = production_secret_store_for_db(Arc::clone(&db)).await;
    assert_eq!(
        consume_secret(&secret_store, &scope.resource, &access_secret).await,
        "production-secret-before-rebuild"
    );
    let other_scope = auth_scope("bob");
    assert!(
        matches!(
            secret_store
                .lease_once(&other_scope.resource, &access_secret)
                .await,
            Err(SecretStoreError::UnknownSecret { .. })
        ),
        "a leaked manual-token handle must not be leaseable from another caller scope"
    );

    drop(services);
    handle.shutdown().await;

    let (rebuilt_notifier, rebuilt_handle) = live_wake_notifier();
    let rebuilt_services = build_production_services_for_db(
        Arc::clone(&db),
        dir.path().join("events-rebuilt.db").to_string_lossy(),
        rebuilt_notifier,
    )
    .await;
    let rebuilt_product_auth = rebuilt_services
        .product_auth
        .as_ref()
        .expect("rebuilt production composes product auth");
    let rebuilt_account = rebuilt_product_auth
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            submitted.account_id,
        ))
        .await
        .expect("rebuilt account lookup should succeed")
        .expect("manual-token account should survive production service rebuild");
    assert_eq!(rebuilt_account.access_secret.as_ref(), Some(&access_secret));

    let rebuilt_secret_store = production_secret_store_for_db(db).await;
    assert_eq!(
        consume_secret(&rebuilt_secret_store, &scope.resource, &access_secret).await,
        "production-secret-before-rebuild"
    );

    rebuilt_handle.shutdown().await;
}

#[tokio::test]
async fn production_manual_token_resubmit_rotates_secret_and_removes_old_handle() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let services = build_production_services_for_db(
        Arc::clone(&db),
        dir.path().join("events.db").to_string_lossy(),
        notifier,
    )
    .await;

    let product_auth = services
        .product_auth
        .as_ref()
        .expect("production composes product auth");
    let scope = auth_scope("alice");
    let provider = AuthProviderId::new("manual-provider").unwrap();
    let label = CredentialAccountLabel::new("rotating manual").unwrap();

    let first_challenge = product_auth
        .request_manual_token_setup(RebornManualTokenSetupRequest::new(
            scope.clone(),
            provider.clone(),
            label.clone(),
            AuthContinuationRef::SetupOnly,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        ))
        .await
        .expect("first manual-token setup should create challenge");
    let first_submit = product_auth
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            scope.clone(),
            first_challenge.interaction_id,
            SecretString::from("production-secret-before-rotation"),
        ))
        .await
        .expect("first manual-token submit should persist secret");
    let first_account = product_auth
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            first_submit.account_id,
        ))
        .await
        .expect("first account lookup should succeed")
        .expect("first manual-token submit should create an account");
    let first_secret = first_account
        .access_secret
        .clone()
        .expect("first account should reference a secret handle");

    let second_challenge = product_auth
        .request_manual_token_setup(RebornManualTokenSetupRequest::new(
            scope.clone(),
            provider,
            label,
            AuthContinuationRef::SetupOnly,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        ))
        .await
        .expect("second manual-token setup should create challenge");
    let second_submit = product_auth
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            scope.clone(),
            second_challenge.interaction_id,
            SecretString::from("production-secret-after-rotation"),
        ))
        .await
        .expect("second manual-token submit should rotate reusable account");
    assert_eq!(
        second_submit.account_id, first_submit.account_id,
        "same provider/label/user should reuse the existing manual-token account"
    );
    let second_account = product_auth
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            second_submit.account_id,
        ))
        .await
        .expect("second account lookup should succeed")
        .expect("second manual-token submit should keep the account");
    let second_secret = second_account
        .access_secret
        .clone()
        .expect("rotated account should reference a secret handle");
    assert_ne!(
        first_secret, second_secret,
        "resubmitting a manual token should rotate to the new interaction-scoped handle"
    );

    let secret_store = production_secret_store_for_db(db).await;
    assert!(
        matches!(
            secret_store.metadata(&scope.resource, &first_secret).await,
            Ok(None)
        ),
        "rotation should delete the old manual-token secret handle"
    );
    assert!(
        matches!(
            secret_store
                .lease_once(&scope.resource, &first_secret)
                .await,
            Err(SecretStoreError::UnknownSecret { .. })
        ),
        "old manual-token material must not remain leaseable after rotation"
    );
    assert_eq!(
        consume_secret(&secret_store, &scope.resource, &second_secret).await,
        "production-secret-after-rotation"
    );

    handle.shutdown().await;
}
