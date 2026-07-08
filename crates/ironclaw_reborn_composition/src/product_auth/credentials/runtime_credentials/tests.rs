use chrono::Utc;
use ironclaw_auth::{
    CredentialAccountLabel, CredentialAccountService, CredentialOwnership,
    InMemoryAuthProductServices, NewCredentialAccount,
};
use ironclaw_host_api::{
    AgentId, CredentialStageError, ExtensionId, InvocationId, MissionId, ProjectId, ResourceScope,
    RuntimeCredentialAccountProviderId, RuntimeCredentialAccountSetup,
    RuntimeCredentialAuthRequirement, SecretHandle, TenantId, ThreadId, UserId,
};
use ironclaw_secrets::{
    InMemorySecretStore, SecretLease, SecretLeaseId, SecretMaterial, SecretMetadata, SecretStore,
    SecretStoreError,
};

use super::*;

mod duplicate_selection;

fn resolver_with_accounts(
    accounts: Arc<InMemoryAuthProductServices>,
) -> ProductAuthRuntimeCredentialResolver {
    ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new_with_visibility(
            accounts,
            Arc::new(crate::extension_host::gsuite::GsuiteRuntimeCredentialAccountVisibilityPolicy),
        ),
    ))
}

fn resolver_with_host_managed_nearai_scope(
    accounts: Arc<InMemoryAuthProductServices>,
    host_scope: AuthProductScope,
) -> ProductAuthRuntimeCredentialResolver {
    use super::host_managed_fallback::{
        HostManagedCredentialFallbackRule, HostManagedRuntimeCredentialAccountSelector,
    };

    let selector: Arc<dyn RuntimeCredentialAccountSelectionService> = Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new_with_visibility(
            accounts,
            Arc::new(crate::extension_host::gsuite::GsuiteRuntimeCredentialAccountVisibilityPolicy),
        ),
    );
    let fallback = HostManagedCredentialFallbackRule::new(
        AuthProviderId::new("nearai").unwrap(),
        ExtensionId::new("nearai").unwrap(),
        host_scope,
    );
    ProductAuthRuntimeCredentialResolver::new(Arc::new(
        HostManagedRuntimeCredentialAccountSelector::new(selector, fallback),
    ))
}

fn resolver_with_refresh(
    accounts: Arc<InMemoryAuthProductServices>,
) -> ProductAuthRuntimeCredentialResolver {
    ProductAuthRuntimeCredentialResolver::new_with_refresh(
        Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new_with_visibility(
                accounts.clone(),
                Arc::new(
                    crate::extension_host::gsuite::GsuiteRuntimeCredentialAccountVisibilityPolicy,
                ),
            ),
        ),
        Arc::new(ProductAuthRuntimeCredentialAccountRefresher::new(
            Arc::new(TestRuntimeCredentialRefreshPort(accounts)),
            Arc::new(InMemorySecretStore::new()),
        )),
    )
}

struct TestRuntimeCredentialRefreshPort(Arc<InMemoryAuthProductServices>);

#[async_trait::async_trait]
impl RuntimeCredentialAccountRefreshPort for TestRuntimeCredentialRefreshPort {
    async fn refresh_credential_account(
        &self,
        request: CredentialRefreshRequest,
    ) -> Result<CredentialRefreshReport, AuthProductError> {
        self.0.refresh_account(request).await
    }
}

struct MetadataUnavailableSecretStore {
    inner: Arc<InMemorySecretStore>,
}

#[async_trait::async_trait]
impl SecretStore for MetadataUnavailableSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
        expires_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<SecretMetadata, SecretStoreError> {
        self.inner.put(scope, handle, material, expires_at).await
    }

    async fn metadata(
        &self,
        _scope: &ResourceScope,
        _handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        Err(SecretStoreError::StoreUnavailable {
            reason: "metadata unavailable for test".to_string(),
        })
    }

    async fn metadata_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
        self.inner.metadata_for_scope(scope).await
    }

    async fn delete(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        self.inner.delete(scope, handle).await
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        self.inner.lease_once(scope, handle).await
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        self.inner.consume(scope, lease_id).await
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        self.inner.revoke(scope, lease_id).await
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        self.inner.leases_for_scope(scope).await
    }
}

fn selector_for(
    accounts: Arc<InMemoryAuthProductServices>,
) -> ProductAuthRuntimeCredentialAccountSelector {
    ProductAuthRuntimeCredentialAccountSelector::new_with_visibility(
        accounts,
        Arc::new(crate::extension_host::gsuite::GsuiteRuntimeCredentialAccountVisibilityPolicy),
    )
}

/// Owner-level (`Api` surface) auth scope for `user` with a fresh invocation.
fn owner_auth_scope(user: &str) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope::local_default(UserId::new(user).unwrap(), InvocationId::new()).unwrap(),
        AuthSurface::Api,
    )
}

/// Fixture builder for a configured `NewCredentialAccount`, collapsing the
/// ten-field literal these tests would otherwise repeat verbatim. Defaults to a
/// `Configured`/`UserReusable` account with an access secret and no scopes;
/// setters override only what a case actually exercises.
struct ConfiguredAccount {
    inner: NewCredentialAccount,
}

impl ConfiguredAccount {
    fn new(scope: AuthProductScope, provider: &str) -> Self {
        Self {
            inner: NewCredentialAccount {
                scope,
                provider: AuthProviderId::new(provider).unwrap(),
                label: CredentialAccountLabel::new(format!("{provider} account")).unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new(format!("{provider}_access")).unwrap()),
                refresh_secret: None,
                scopes: Vec::new(),
            },
        }
    }

    fn label(mut self, label: &str) -> Self {
        self.inner.label = CredentialAccountLabel::new(label).unwrap();
        self
    }

    fn status(mut self, status: CredentialAccountStatus) -> Self {
        self.inner.status = status;
        self
    }

    fn ownership(mut self, ownership: CredentialOwnership) -> Self {
        self.inner.ownership = ownership;
        self
    }

    fn owner_extension(mut self, extension: &str) -> Self {
        self.inner.owner_extension = Some(ExtensionId::new(extension).unwrap());
        self
    }

    fn granted_extensions(mut self, extensions: Vec<ExtensionId>) -> Self {
        self.inner.granted_extensions = extensions;
        self
    }

    /// Override the default `<provider>_access` access secret. Pass `None` to
    /// model a `Configured`-but-secretless (corrupt) account.
    fn access_secret(mut self, handle: Option<SecretHandle>) -> Self {
        self.inner.access_secret = handle;
        self
    }

    fn refresh_secret(mut self, handle: SecretHandle) -> Self {
        self.inner.refresh_secret = Some(handle);
        self
    }

    fn scopes(mut self, scopes: &[&str]) -> Self {
        self.inner.scopes = scopes
            .iter()
            .map(|scope| ProviderScope::new(*scope).unwrap())
            .collect();
        self
    }

    async fn create(self, accounts: &InMemoryAuthProductServices) -> CredentialAccount {
        accounts.create_account(self.inner).await.unwrap()
    }
}

#[tokio::test]
async fn binding_selection_ignores_provider_scope_gate() {
    // Defect A (A1): the OAuth bind lookup must find the owner's existing
    // google account even when it lacks the newly requested scope, so a
    // reconnect that grants a new scope UPDATES the existing account instead of
    // forking a duplicate.
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let auth_scope = owner_auth_scope("alice");
    let created = ConfiguredAccount::new(auth_scope.clone(), "google")
        .scopes(&["https://www.googleapis.com/auth/gmail.send"])
        .create(&accounts)
        .await;
    let selector = selector_for(accounts);

    // Lookup for a calendar reconnect — the account holds only gmail.send.
    let bound = selector
        .select_configured_account_for_binding(
            CredentialAccountSelectionRequest::new(
                auth_scope.clone(),
                AuthProviderId::new("google").unwrap(),
            )
            .for_extension(ExtensionId::new("google-calendar").unwrap()),
            auth_scope,
        )
        .await
        .expect("binding must find the existing account despite the missing scope");

    assert_eq!(bound.id, created.id);
}

#[tokio::test]
async fn runtime_selection_still_enforces_provider_scope_gate() {
    // Defect A guard: relaxing the scope gate is scoped to BINDING only. Runtime
    // resolution keeps the gate, so an account lacking the requested scope is
    // CredentialMissing (never serves a token for a scope it does not hold).
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let auth_scope = owner_auth_scope("alice");
    ConfiguredAccount::new(auth_scope.clone(), "google")
        .scopes(&["https://www.googleapis.com/auth/gmail.send"])
        .create(&accounts)
        .await;
    let selector = selector_for(accounts);

    let error = selector
        .select_unique_configured_runtime_account(RuntimeCredentialAccountSelectionRequest::new(
            CredentialAccountSelectionRequest::new(
                auth_scope.clone(),
                AuthProviderId::new("google").unwrap(),
            )
            .for_extension(ExtensionId::new("google-drive").unwrap()),
            auth_scope,
            RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            vec![ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap()],
        ))
        .await
        .unwrap_err();

    assert_eq!(error, AuthProductError::CredentialMissing);
}

#[tokio::test]
async fn binding_does_not_cross_session_boundary() {
    // session_id is path-segmenting for the bind/update WRITE path: an account
    // stored under one session must not be bound by a flow targeting a
    // different (or no) session, or the callback — which updates the account at
    // the flow scope's session path — could never write it. `accounts_for_owner`
    // wildcards session when the flow session is `None`, so the bind selection
    // must re-impose exact session equality.
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let session_scope = owner_auth_scope("alice")
        .with_session_id(ironclaw_auth::AuthSessionId::new("sess-a").unwrap());
    ConfiguredAccount::new(session_scope, "google")
        .create(&accounts)
        .await;
    let selector = selector_for(accounts);

    // A reconnect carrying NO session must not bind the session-scoped account.
    let no_session = owner_auth_scope("alice");
    let error = selector
        .select_configured_account_for_binding(
            CredentialAccountSelectionRequest::new(
                no_session.clone(),
                AuthProviderId::new("google").unwrap(),
            )
            .for_extension(ExtensionId::new("google-calendar").unwrap()),
            no_session,
        )
        .await
        .unwrap_err();

    assert_eq!(error, AuthProductError::CredentialMissing);
}

#[tokio::test]
async fn binding_matches_account_within_same_session() {
    // The flip side of `binding_does_not_cross_session_boundary`: a reconnect on
    // the SAME session still binds the owner's existing account.
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let session_scope = owner_auth_scope("alice")
        .with_session_id(ironclaw_auth::AuthSessionId::new("sess-a").unwrap());
    let created = ConfiguredAccount::new(session_scope.clone(), "google")
        .create(&accounts)
        .await;
    let selector = selector_for(accounts);

    let bound = selector
        .select_configured_account_for_binding(
            CredentialAccountSelectionRequest::new(
                session_scope.clone(),
                AuthProviderId::new("google").unwrap(),
            )
            .for_extension(ExtensionId::new("google-calendar").unwrap()),
            session_scope,
        )
        .await
        .expect("same-session reconnect must bind the existing account");

    assert_eq!(bound.id, created.id);
}

#[tokio::test]
async fn binding_does_not_cross_surface_boundary() {
    // surface is path-segmenting for the bind/update WRITE path exactly like
    // session: durable account records live under a per-surface path, and the
    // callback updates the account at the flow scope's surface path.
    // `accounts_for_owner` enumerates EVERY surface, so the bind selection must
    // re-impose exact surface equality — otherwise it could select an account
    // stored on another surface that the callback can never read (a spurious
    // CredentialMissing that aborts the reconnect).
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let web_scope = AuthProductScope::new(
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap(),
        AuthSurface::Web,
    );
    ConfiguredAccount::new(web_scope, "google")
        .create(&accounts)
        .await;
    let selector = selector_for(accounts);

    // A reconnect on the `Api` surface must not bind the `Web`-surface account.
    let api_scope = owner_auth_scope("alice");
    let error = selector
        .select_configured_account_for_binding(
            CredentialAccountSelectionRequest::new(
                api_scope.clone(),
                AuthProviderId::new("google").unwrap(),
            )
            .for_extension(ExtensionId::new("google-calendar").unwrap()),
            api_scope,
        )
        .await
        .unwrap_err();

    assert_eq!(error, AuthProductError::CredentialMissing);
}

#[tokio::test]
async fn binding_selection_enforces_requester_visibility() {
    // The binding path skips the provider-scope gate but still relies on
    // `finalize_selection` to enforce requester authorization: an
    // extension-owned account must not be bound by an unrelated third-party
    // requester. Lock that on `select_configured_account_for_binding` directly
    // (existing third-party denial coverage exercises runtime selection).
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let auth_scope = owner_auth_scope("alice");
    ConfiguredAccount::new(auth_scope.clone(), "google")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("google-drive")
        .create(&accounts)
        .await;
    let selector = selector_for(accounts);

    let error = selector
        .select_configured_account_for_binding(
            CredentialAccountSelectionRequest::new(
                auth_scope.clone(),
                AuthProviderId::new("google").unwrap(),
            )
            .for_extension(ExtensionId::new("github").unwrap()),
            auth_scope,
        )
        .await
        .unwrap_err();

    assert_eq!(error, AuthProductError::CrossScopeDenied);
}

#[tokio::test]
async fn runtime_resolution_finds_shared_admin_account_across_thread() {
    // The cross-thread ownership guarantee is not limited to UserReusable and
    // ExtensionOwned: a SharedAdminManaged account authorized in one thread must
    // resolve for a granted requester from any other thread of the same owner.
    // `account_visible_from_runtime_scope` treats thread/mission/session as
    // non-ownership axes for every ownership type, so a regression to
    // thread-bound resolution here would otherwise slip through.
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let requester = ExtensionId::new("gmail").unwrap();
    let gmail_scope = "https://www.googleapis.com/auth/gmail.readonly";
    let mut thread_a = owner_auth_scope("alice");
    thread_a.resource.thread_id = Some(ThreadId::new("thread-a").unwrap());
    let created = ConfiguredAccount::new(thread_a, "google")
        .ownership(CredentialOwnership::SharedAdminManaged)
        .granted_extensions(vec![requester.clone()])
        .scopes(&[gmail_scope])
        .create(&accounts)
        .await;
    let selector = selector_for(accounts);

    // Runtime resolution looks the owner up at owner granularity (thread
    // stripped) and carries the live thread only as the runtime/visibility
    // scope — exactly as the production resolvers do. Resolving from thread-b
    // must still find the account authorized in thread-a.
    let owner_lookup = owner_auth_scope("alice");
    let mut runtime_thread_b = owner_auth_scope("alice");
    runtime_thread_b.resource.thread_id = Some(ThreadId::new("thread-b").unwrap());
    let resolved = selector
        .select_unique_configured_runtime_account(RuntimeCredentialAccountSelectionRequest::new(
            CredentialAccountSelectionRequest::new(
                owner_lookup,
                AuthProviderId::new("google").unwrap(),
            )
            .for_extension(requester),
            runtime_thread_b,
            RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            vec![ProviderScope::new(gmail_scope).unwrap()],
        ))
        .await
        .expect("shared-admin account must resolve from a different thread");

    assert_eq!(resolved.id, created.id);
}

#[tokio::test]
async fn resolver_resolves_shared_admin_account_from_new_thread() {
    // The resolver must preserve the same cross-thread contract as the
    // selector: a SharedAdminManaged account explicitly granted to a requester
    // in one thread remains resolvable from a different thread of the same
    // owner.
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let requester = ExtensionId::new("gmail").unwrap();
    let gmail_scope = "https://www.googleapis.com/auth/gmail.readonly";
    let mut thread_a = owner_auth_scope("alice");
    thread_a.resource.thread_id = Some(ThreadId::new("thread-a").unwrap());
    let access_secret = SecretHandle::new("shared-admin-google-access").unwrap();
    ConfiguredAccount::new(thread_a, "google")
        .ownership(CredentialOwnership::SharedAdminManaged)
        .granted_extensions(vec![requester.clone()])
        .access_secret(Some(access_secret.clone()))
        .scopes(&[gmail_scope])
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);

    let mut thread_b = owner_auth_scope("alice");
    thread_b.resource.thread_id = Some(ThreadId::new("thread-b").unwrap());
    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &thread_b.resource,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[gmail_scope.to_string()],
            requester_extension: &requester,
        })
        .await
        .expect("shared-admin account must resolve from a new thread");

    assert_eq!(resolved.handle, access_secret);
}

#[tokio::test]
async fn resolver_returns_configured_product_auth_access_secret() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("github_manual_access").unwrap();
    ConfiguredAccount::new(auth_scope, "github")
        .access_secret(Some(access_secret.clone()))
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, scope);
}

#[tokio::test]
async fn resolver_uses_host_managed_nearai_account_for_nearai_requester() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let host_scope = owner_auth_scope("reborn-cli");
    let user_scope = owner_auth_scope("alice");
    let host_access_secret = SecretHandle::new("host_nearai_access").unwrap();
    ConfiguredAccount::new(host_scope.clone(), "nearai")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("nearai")
        .access_secret(Some(host_access_secret.clone()))
        .create(&accounts)
        .await;
    let resolver = resolver_with_host_managed_nearai_scope(accounts, host_scope.clone());

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &user_scope.resource,
            provider: &RuntimeCredentialAccountProviderId::new("nearai").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("nearai").unwrap(),
        })
        .await
        .expect("NEAR AI MCP should use the host-managed NEAR AI credential");

    assert_eq!(resolved.handle, host_access_secret);
    assert_eq!(resolved.scope, host_scope.resource);
}

#[tokio::test]
async fn resolver_uses_tenant_agent_level_host_managed_nearai_account_for_project_scope() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let mut host_scope = owner_auth_scope("reborn-cli");
    host_scope.resource.project_id = None;
    let mut user_scope = owner_auth_scope("alice");
    user_scope.resource.project_id = Some(ProjectId::new("project-a").unwrap());
    let host_access_secret = SecretHandle::new("host_nearai_access").unwrap();
    ConfiguredAccount::new(host_scope.clone(), "nearai")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("nearai")
        .access_secret(Some(host_access_secret.clone()))
        .create(&accounts)
        .await;
    let resolver = resolver_with_host_managed_nearai_scope(accounts, host_scope.clone());

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &user_scope.resource,
            provider: &RuntimeCredentialAccountProviderId::new("nearai").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("nearai").unwrap(),
        })
        .await
        .expect("tenant/agent-level host-managed NEAR AI credential should cover project scopes");

    assert_eq!(resolved.handle, host_access_secret);
    assert_eq!(resolved.scope, host_scope.resource);
}

#[tokio::test]
async fn resolver_prefers_user_nearai_account_over_host_managed_nearai_account() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let host_scope = owner_auth_scope("reborn-cli");
    let user_scope = owner_auth_scope("alice");
    let host_access_secret = SecretHandle::new("host_nearai_access").unwrap();
    let user_access_secret = SecretHandle::new("alice_nearai_access").unwrap();
    ConfiguredAccount::new(host_scope.clone(), "nearai")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("nearai")
        .access_secret(Some(host_access_secret))
        .create(&accounts)
        .await;
    ConfiguredAccount::new(user_scope.clone(), "nearai")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("nearai")
        .access_secret(Some(user_access_secret.clone()))
        .create(&accounts)
        .await;
    let resolver = resolver_with_host_managed_nearai_scope(accounts, host_scope);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &user_scope.resource,
            provider: &RuntimeCredentialAccountProviderId::new("nearai").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("nearai").unwrap(),
        })
        .await
        .expect("user-owned NEAR AI credential should take precedence");

    assert_eq!(resolved.handle, user_access_secret);
    assert_eq!(resolved.scope, user_scope.resource);
}

#[tokio::test]
async fn resolver_does_not_share_host_managed_nearai_account_with_other_requester() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let host_scope = owner_auth_scope("reborn-cli");
    let user_scope = owner_auth_scope("alice");
    ConfiguredAccount::new(host_scope.clone(), "nearai")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("nearai")
        .create(&accounts)
        .await;
    let resolver = resolver_with_host_managed_nearai_scope(accounts, host_scope);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &user_scope.resource,
            provider: &RuntimeCredentialAccountProviderId::new("nearai").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("third-party").unwrap(),
        })
        .await
        .expect_err("host-managed NEAR AI fallback is scoped to the nearai requester");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_does_not_share_host_managed_nearai_account_across_tenant() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let host_scope = owner_auth_scope("reborn-cli");
    let mut other_tenant_scope = owner_auth_scope("alice");
    other_tenant_scope.resource.tenant_id = TenantId::new("other-tenant").unwrap();
    ConfiguredAccount::new(host_scope.clone(), "nearai")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("nearai")
        .create(&accounts)
        .await;
    let resolver = resolver_with_host_managed_nearai_scope(accounts, host_scope);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &other_tenant_scope.resource,
            provider: &RuntimeCredentialAccountProviderId::new("nearai").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("nearai").unwrap(),
        })
        .await
        .expect_err("host-managed NEAR AI fallback must stay tenant-scoped");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_does_not_share_host_managed_nearai_account_across_agent() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let host_scope = owner_auth_scope("reborn-cli");
    let mut other_agent_scope = owner_auth_scope("alice");
    other_agent_scope.resource.agent_id = Some(AgentId::new("other-agent").unwrap());
    ConfiguredAccount::new(host_scope.clone(), "nearai")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("nearai")
        .create(&accounts)
        .await;
    let resolver = resolver_with_host_managed_nearai_scope(accounts, host_scope);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &other_agent_scope.resource,
            provider: &RuntimeCredentialAccountProviderId::new("nearai").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("nearai").unwrap(),
        })
        .await
        .expect_err("host-managed NEAR AI fallback must stay agent-scoped");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_does_not_share_project_scoped_host_managed_nearai_account_across_project() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let mut host_scope = owner_auth_scope("reborn-cli");
    host_scope.resource.project_id = Some(ProjectId::new("host-project").unwrap());
    let mut other_project_scope = owner_auth_scope("alice");
    other_project_scope.resource.project_id = Some(ProjectId::new("other-project").unwrap());
    ConfiguredAccount::new(host_scope.clone(), "nearai")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("nearai")
        .create(&accounts)
        .await;
    let resolver = resolver_with_host_managed_nearai_scope(accounts, host_scope);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &other_project_scope.resource,
            provider: &RuntimeCredentialAccountProviderId::new("nearai").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("nearai").unwrap(),
        })
        .await
        .expect_err("project-scoped host-managed NEAR AI fallback must stay project-scoped");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_uses_project_scoped_host_managed_nearai_account_for_same_project() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let mut host_scope = owner_auth_scope("reborn-cli");
    host_scope.resource.project_id = Some(ProjectId::new("host-project").unwrap());
    let mut same_project_scope = owner_auth_scope("alice");
    same_project_scope.resource.project_id = Some(ProjectId::new("host-project").unwrap());
    let host_access_secret = SecretHandle::new("host_nearai_access").unwrap();
    ConfiguredAccount::new(host_scope.clone(), "nearai")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("nearai")
        .access_secret(Some(host_access_secret.clone()))
        .create(&accounts)
        .await;
    let resolver = resolver_with_host_managed_nearai_scope(accounts, host_scope.clone());

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &same_project_scope.resource,
            provider: &RuntimeCredentialAccountProviderId::new("nearai").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("nearai").unwrap(),
        })
        .await
        .expect("project-scoped host-managed NEAR AI fallback should resolve in the same project");

    assert_eq!(resolved.handle, host_access_secret);
    assert_eq!(resolved.scope, host_scope.resource);
}

#[tokio::test]
async fn resolver_does_not_share_host_managed_scope_for_other_provider() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let host_scope = owner_auth_scope("reborn-cli");
    let user_scope = owner_auth_scope("alice");
    ConfiguredAccount::new(host_scope.clone(), "github")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("github")
        .create(&accounts)
        .await;
    let resolver = resolver_with_host_managed_nearai_scope(accounts, host_scope);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &user_scope.resource,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .expect_err("host-managed fallback must not apply to non-NEAR AI providers");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_refreshes_oauth_account_before_staging_access_secret() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let stale_access = SecretHandle::new("google_stale_access").unwrap();
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    ConfiguredAccount::new(auth_scope, "google")
        .access_secret(Some(stale_access.clone()))
        .refresh_secret(SecretHandle::new("google_refresh").unwrap())
        .scopes(&["https://www.googleapis.com/auth/drive.readonly"])
        .create(&accounts)
        .await;
    let resolver = resolver_with_refresh(accounts.clone());

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[drive_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect("OAuth runtime credentials should refresh before staging");

    assert_eq!(resolved.scope, scope);
    assert_ne!(resolved.handle, stale_access);
    assert!(
        resolved
            .handle
            .as_str()
            .starts_with("oauth-refreshed-access")
    );
}

#[tokio::test]
async fn resolver_refreshes_gsuite_owned_account_with_owner_authority_for_sibling_requester() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let stale_access = SecretHandle::new("google_stale_gsuite_access").unwrap();
    let calendar_scope =
        ProviderScope::new("https://www.googleapis.com/auth/calendar.readonly").unwrap();
    ConfiguredAccount::new(auth_scope, "google")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("google-drive")
        .access_secret(Some(stale_access.clone()))
        .refresh_secret(SecretHandle::new("google_gsuite_refresh").unwrap())
        .scopes(&["https://www.googleapis.com/auth/calendar.readonly"])
        .create(&accounts)
        .await;
    let resolver = resolver_with_refresh(accounts.clone());

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[calendar_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-calendar").unwrap(),
        })
        .await
        .expect("GSuite siblings should refresh through the selected account owner");

    assert_ne!(resolved.handle, stale_access);
    assert!(
        resolved
            .handle
            .as_str()
            .starts_with("oauth-refreshed-access")
    );
}

#[tokio::test]
async fn resolver_refreshes_oauth_account_for_each_runtime_staging() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    ConfiguredAccount::new(auth_scope, "google")
        .access_secret(Some(SecretHandle::new("google_stale_access_once").unwrap()))
        .refresh_secret(SecretHandle::new("google_refresh_once").unwrap())
        .scopes(&["https://www.googleapis.com/auth/drive.readonly"])
        .create(&accounts)
        .await;
    let resolver = resolver_with_refresh(accounts.clone());
    let provider = RuntimeCredentialAccountProviderId::new("google").unwrap();
    let setup = RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() };
    let provider_scopes = vec![drive_scope.as_str().to_string()];
    let requester_extension = ExtensionId::new("google-drive").unwrap();

    let first = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &provider,
            setup: &setup,
            provider_scopes: &provider_scopes,
            requester_extension: &requester_extension,
        })
        .await
        .expect("first OAuth staging refreshes");
    let second = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &provider,
            setup: &setup,
            provider_scopes: &provider_scopes,
            requester_extension: &requester_extension,
        })
        .await
        .expect("second OAuth staging refreshes again");

    assert_ne!(second.handle, first.handle);
}

#[tokio::test]
async fn resolver_stages_oauth_access_secret_when_refresh_secret_is_absent() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("google_access_without_refresh").unwrap();
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    ConfiguredAccount::new(auth_scope, "google")
        .access_secret(Some(access_secret.clone()))
        .scopes(&["https://www.googleapis.com/auth/drive.readonly"])
        .create(&accounts)
        .await;
    let resolver = resolver_with_refresh(accounts.clone());

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[drive_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect("configured OAuth access token should still stage without a refresh token");

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, scope);
}

#[tokio::test]
async fn resolver_stages_oauth_access_secret_when_proactive_refresh_backend_is_unavailable() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("google_access_refresh_backend_down").unwrap();
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    let account = ConfiguredAccount::new(auth_scope, "google")
        .access_secret(Some(access_secret.clone()))
        .refresh_secret(SecretHandle::new("google_refresh").unwrap())
        .scopes(&["https://www.googleapis.com/auth/drive.readonly"])
        .create(&accounts)
        .await;
    accounts.fail_next_refresh_backend_for_tests(account.id);
    assert!(
        accounts.has_pending_refresh_backend_failure_for_tests(account.id),
        "test must start with a staged backend refresh failure"
    );
    let secret_store = Arc::new(InMemorySecretStore::new());
    secret_store
        .put(
            scope.clone(),
            access_secret.clone(),
            ironclaw_secrets::SecretMaterial::from("[placeholder]".to_string()),
            None,
        )
        .await
        .expect("seed access-token metadata without expiry");
    let resolver = resolver_with_refresh_and_store(accounts.clone(), secret_store);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[drive_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect("proactive refresh backend outage should not fail configured token staging");

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, scope);
    assert!(
        !accounts.has_pending_refresh_backend_failure_for_tests(account.id),
        "resolver must attempt refresh and consume the staged backend failure"
    );
}

#[tokio::test]
async fn resolver_propagates_backend_error_when_stale_access_token_cannot_refresh() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("google_expired_access_refresh_backend_down").unwrap();
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    let account = ConfiguredAccount::new(auth_scope, "google")
        .access_secret(Some(access_secret.clone()))
        .refresh_secret(SecretHandle::new("google_refresh_backend_down_expired").unwrap())
        .scopes(&["https://www.googleapis.com/auth/drive.readonly"])
        .create(&accounts)
        .await;
    accounts.fail_next_refresh_backend_for_tests(account.id);

    let secret_store = Arc::new(InMemorySecretStore::new());
    secret_store
        .put(
            account.scope.resource.clone(),
            access_secret,
            ironclaw_secrets::SecretMaterial::from("[placeholder]".to_string()),
            Some(Utc::now() - chrono::Duration::minutes(1)),
        )
        .await
        .expect("seed expired access-token metadata");
    let resolver = resolver_with_refresh_and_store(accounts.clone(), secret_store);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[drive_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect_err("known-stale access token plus backend refresh failure is backend, not auth");

    assert_eq!(error, CredentialStageError::Backend);
}

#[tokio::test]
async fn resolver_propagates_backend_error_when_access_secret_metadata_is_missing() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("google_missing_access_metadata").unwrap();
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    let account = ConfiguredAccount::new(auth_scope, "google")
        .access_secret(Some(access_secret))
        .refresh_secret(SecretHandle::new("google_refresh_missing_metadata").unwrap())
        .scopes(&["https://www.googleapis.com/auth/drive.readonly"])
        .create(&accounts)
        .await;
    accounts.fail_next_refresh_backend_for_tests(account.id);
    let resolver = resolver_with_refresh_and_store(accounts, Arc::new(InMemorySecretStore::new()));

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[drive_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect_err("missing access-token metadata must not fall back after refresh failure");

    assert_eq!(error, CredentialStageError::Backend);
}

#[tokio::test]
async fn resolver_propagates_backend_error_when_access_secret_metadata_is_unreadable() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("google_unreadable_access_metadata").unwrap();
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    let account = ConfiguredAccount::new(auth_scope, "google")
        .access_secret(Some(access_secret.clone()))
        .refresh_secret(SecretHandle::new("google_refresh_unreadable_metadata").unwrap())
        .scopes(&["https://www.googleapis.com/auth/drive.readonly"])
        .create(&accounts)
        .await;
    accounts.fail_next_refresh_backend_for_tests(account.id);
    let inner_store = Arc::new(InMemorySecretStore::new());
    inner_store
        .put(
            scope.clone(),
            access_secret,
            ironclaw_secrets::SecretMaterial::from("[placeholder]".to_string()),
            Some(Utc::now() + chrono::Duration::hours(1)),
        )
        .await
        .expect("seed access-token metadata before wrapping unreadable store");
    let resolver = resolver_with_refresh_and_store(
        accounts,
        Arc::new(MetadataUnavailableSecretStore { inner: inner_store }),
    );

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[drive_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect_err("unreadable access-token metadata must fail closed after refresh failure");

    assert_eq!(error, CredentialStageError::Backend);
}

#[tokio::test]
async fn resolver_maps_oauth_refresh_failure_to_auth_required() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    let account = ConfiguredAccount::new(auth_scope, "google")
        .access_secret(Some(SecretHandle::new("google_stale_access").unwrap()))
        .refresh_secret(SecretHandle::new("google_refresh").unwrap())
        .scopes(&["https://www.googleapis.com/auth/drive.readonly"])
        .create(&accounts)
        .await;
    accounts.fail_next_refresh_for_tests(account.id);
    let resolver = resolver_with_refresh(accounts.clone());

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[drive_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect_err("stale OAuth access token must not be staged after refresh failure");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_accepts_unscoped_github_manual_token_for_scoped_runtime_request() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("github_manual_access").unwrap();
    ConfiguredAccount::new(auth_scope, "github")
        .access_secret(Some(access_secret.clone()))
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);
    let required_scopes = vec!["repo".to_string()];

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &required_scopes,
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .expect("GitHub PAT scopes are encoded in the token and cannot be introspected");

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, scope);
}

#[tokio::test]
async fn resolver_does_not_use_reusable_account_from_different_user() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let alice_scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let admin_scope =
        ResourceScope::local_default(UserId::new("admin").unwrap(), InvocationId::new()).unwrap();
    ConfiguredAccount::new(
        AuthProductScope::new(alice_scope, AuthSurface::Api),
        "google",
    )
    .access_secret(Some(SecretHandle::new("alice-google-access").unwrap()))
    .create(&accounts)
    .await;
    let resolver = resolver_with_accounts(accounts);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &admin_scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("gmail").unwrap(),
        })
        .await
        .expect_err("admin must not resolve alice's reusable account");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_matches_callback_setup_account_from_runtime_invocation() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let mut setup_scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    setup_scope.thread_id = Some(ThreadId::new("thread-auth-1").unwrap());
    let mut runtime_scope = setup_scope.clone();
    runtime_scope.invocation_id = InvocationId::new();
    let access_secret = SecretHandle::new("github_manual_access").unwrap();
    ConfiguredAccount::new(
        AuthProductScope::new(setup_scope.clone(), AuthSurface::Callback),
        "github",
    )
    .access_secret(Some(access_secret.clone()))
    .create(&accounts)
    .await;
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &runtime_scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, setup_scope);
}

#[tokio::test]
async fn resolver_matches_reusable_setup_account_from_new_thread() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let mut setup_scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    setup_scope.thread_id = Some(ThreadId::new("thread-auth-1").unwrap());
    let mut runtime_scope = setup_scope.clone();
    runtime_scope.thread_id = Some(ThreadId::new("thread-auth-2").unwrap());
    runtime_scope.invocation_id = InvocationId::new();
    let access_secret = SecretHandle::new("github_manual_access").unwrap();
    ConfiguredAccount::new(
        AuthProductScope::new(setup_scope.clone(), AuthSurface::Callback),
        "github",
    )
    .access_secret(Some(access_secret.clone()))
    .create(&accounts)
    .await;
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &runtime_scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, setup_scope);
}

#[tokio::test]
async fn resolver_matches_reusable_setup_account_from_new_mission() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let mut setup_scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    setup_scope.mission_id = Some(MissionId::new("mission-auth-1").unwrap());
    let mut runtime_scope = setup_scope.clone();
    runtime_scope.mission_id = Some(MissionId::new("mission-auth-2").unwrap());
    runtime_scope.invocation_id = InvocationId::new();
    let access_secret = SecretHandle::new("github_manual_access").unwrap();
    ConfiguredAccount::new(
        AuthProductScope::new(setup_scope.clone(), AuthSurface::Callback),
        "github",
    )
    .access_secret(Some(access_secret.clone()))
    .create(&accounts)
    .await;
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &runtime_scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, setup_scope);
}

#[tokio::test]
async fn resolver_resolves_extension_owned_account_from_new_thread() {
    // Regression (#4920-follow-up): an ExtensionOwned credential authorized via
    // an OAuth callback in one thread (`thread-auth-1`, `Callback` surface) MUST
    // still resolve when the owning extension runs a tool in a new thread
    // (`thread-auth-2`). Credentials are owned by the user/extension, not the
    // thread they were authorized in. This previously asserted `AuthRequired`,
    // which was the bug: a user lost their auth every time they opened a new
    // chat thread.
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let mut setup_scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    setup_scope.thread_id = Some(ThreadId::new("thread-auth-1").unwrap());
    setup_scope.mission_id = Some(MissionId::new("mission-auth-1").unwrap());
    let mut runtime_scope = setup_scope.clone();
    runtime_scope.thread_id = Some(ThreadId::new("thread-auth-2").unwrap());
    runtime_scope.mission_id = Some(MissionId::new("mission-auth-2").unwrap());
    runtime_scope.invocation_id = InvocationId::new();
    let created = ConfiguredAccount::new(
        AuthProductScope::new(setup_scope, AuthSurface::Callback),
        "github",
    )
    .ownership(CredentialOwnership::ExtensionOwned)
    .owner_extension("github")
    .create(&accounts)
    .await;
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &runtime_scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .expect("extension-owned credential must resolve from a new thread");

    assert_eq!(Some(resolved.handle), created.access_secret);
}

#[tokio::test]
async fn resolver_maps_missing_account_to_auth_required() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let resolver = resolver_with_accounts(accounts);
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap_err();

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_requires_requested_provider_scopes() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    ConfiguredAccount::new(auth_scope, "google")
        .access_secret(Some(SecretHandle::new("google_manual_access").unwrap()))
        .scopes(&["https://www.googleapis.com/auth/gmail.send"])
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);
    let required_scopes = vec!["https://www.googleapis.com/auth/drive".to_string()];

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &required_scopes,
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .unwrap_err();

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_does_not_treat_unscoped_google_account_as_scoped() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    ConfiguredAccount::new(auth_scope, "google")
        .access_secret(Some(SecretHandle::new("google_manual_access").unwrap()))
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);
    let required_scopes = vec!["https://www.googleapis.com/auth/drive".to_string()];

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth {
                scopes: required_scopes.clone(),
            },
            provider_scopes: &required_scopes,
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect_err("unscoped OAuth accounts must not satisfy scoped Google requirements");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_reuses_gsuite_owned_google_account_for_gsuite_requester() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("google-drive-access").unwrap();
    let calendar_scope =
        ProviderScope::new("https://www.googleapis.com/auth/calendar.readonly").unwrap();
    ConfiguredAccount::new(auth_scope, "google")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("google-drive")
        .access_secret(Some(access_secret.clone()))
        .scopes(&["https://www.googleapis.com/auth/calendar.readonly"])
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[calendar_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-calendar").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(resolved.handle, access_secret);
}

#[tokio::test]
async fn resolver_does_not_share_unbound_google_account_with_third_party_requester() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let google_scope =
        ProviderScope::new("https://www.googleapis.com/auth/gmail.readonly").unwrap();
    ConfiguredAccount::new(auth_scope, "google")
        .access_secret(Some(SecretHandle::new("google-access").unwrap()))
        .scopes(&["https://www.googleapis.com/auth/gmail.readonly"])
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[google_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("third-party").unwrap(),
        })
        .await
        .expect_err("third-party requesters need an explicit Google account grant");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_allows_google_account_explicitly_granted_to_third_party_requester() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let requester = ExtensionId::new("third-party").unwrap();
    let access_secret = SecretHandle::new("granted-google-access").unwrap();
    let google_scope =
        ProviderScope::new("https://www.googleapis.com/auth/gmail.readonly").unwrap();
    ConfiguredAccount::new(auth_scope, "google")
        .ownership(CredentialOwnership::SharedAdminManaged)
        .granted_extensions(vec![requester.clone()])
        .access_secret(Some(access_secret.clone()))
        .scopes(&["https://www.googleapis.com/auth/gmail.readonly"])
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[google_scope.as_str().to_string()],
            requester_extension: &requester,
        })
        .await
        .expect("explicit grants should still authorize third-party requesters");

    assert_eq!(resolved.handle, access_secret);
}

#[tokio::test]
async fn resolver_maps_unconfigured_account_status_to_auth_required() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    ConfiguredAccount::new(auth_scope, "github")
        .status(CredentialAccountStatus::PendingSetup)
        .access_secret(None)
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap_err();

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_maps_configured_account_without_access_secret_to_backend() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    ConfiguredAccount::new(auth_scope, "github")
        // Configured but missing secret — data corruption
        .access_secret(None)
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap_err();

    // Data corruption: should be Backend, not AuthRequired (re-auth would not fix it).
    // The durable product-auth store preserves Configured ↔ access_secret=Some,
    // so this state cannot arise from legitimate cleanup or rotation paths.
    assert_eq!(error, CredentialStageError::Backend);
}

#[tokio::test]
async fn activation_preflight_maps_configured_account_without_access_secret_to_backend() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    ConfiguredAccount::new(auth_scope, "github")
        .access_secret(None)
        .create(&accounts)
        .await;
    let selector = ProductAuthRuntimeCredentialAccountSelector::new(accounts);

    let error = missing_runtime_credential_auth_requirements(
        &selector,
        &scope,
        vec![RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: Default::default(),
            requester_extension: ExtensionId::new("github").unwrap(),
            provider_scopes: Vec::new(),
        }],
    )
    .await
    .unwrap_err();

    assert_eq!(error, CredentialStageError::Backend);
}

#[tokio::test]
async fn resolver_uses_most_recent_account_across_multiple_reusable_logins() {
    // Runtime default rule (#auth-gate-reuse): when several reusable,
    // unbound accounts match the same provider — even under different
    // labels — the gate has no interactive picker, so the resolver selects
    // the most-recently-used account rather than failing with
    // `AccountSelectionRequired` (which re-prompted on every call). The
    // setup-time picker controls which one wins by bumping its recency.
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let latest_secret = SecretHandle::new("work-token").unwrap();
    // Two reusable accounts for the same provider under distinct labels.
    // The second one is created later, so it is the most-recently-used.
    ConfiguredAccount::new(auth_scope.clone(), "github")
        .label("personal github")
        .access_secret(Some(SecretHandle::new("personal-token").unwrap()))
        .create(&accounts)
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    ConfiguredAccount::new(auth_scope, "github")
        .label("work github")
        .access_secret(Some(latest_secret.clone()))
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .expect("runtime must resolve to the most-recent reusable account, not re-prompt");

    assert_eq!(resolved.handle, latest_secret);
}

fn resolver_with_refresh_and_store(
    accounts: Arc<InMemoryAuthProductServices>,
    secret_store: Arc<dyn SecretStore>,
) -> ProductAuthRuntimeCredentialResolver {
    ProductAuthRuntimeCredentialResolver::new_with_refresh(
        Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new_with_visibility(
                accounts.clone(),
                Arc::new(
                    crate::extension_host::gsuite::GsuiteRuntimeCredentialAccountVisibilityPolicy,
                ),
            ),
        ),
        Arc::new(ProductAuthRuntimeCredentialAccountRefresher::new(
            Arc::new(TestRuntimeCredentialRefreshPort(accounts)),
            secret_store,
        )),
    )
}

#[tokio::test]
async fn resolver_skips_inline_refresh_when_access_token_is_fresh() {
    // A2: An access secret with expires_at far in the future (> margin) must
    // cause the inline refresh to be SKIPPED entirely. The returned handle must
    // be the original access handle, not an "oauth-refreshed-*" handle.
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("google_fresh_access").unwrap();
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    ConfiguredAccount::new(auth_scope, "google")
        .access_secret(Some(access_secret.clone()))
        .refresh_secret(SecretHandle::new("google_refresh").unwrap())
        .scopes(&["https://www.googleapis.com/auth/drive.readonly"])
        .create(&accounts)
        .await;

    // Pre-populate the secret store with a fresh expiry (1 hour from now).
    let secret_store = Arc::new(InMemorySecretStore::new());
    secret_store
        .put(
            scope.clone(),
            access_secret.clone(),
            ironclaw_secrets::SecretMaterial::from("[placeholder]".to_string()),
            Some(Utc::now() + chrono::Duration::hours(1)),
        )
        .await
        .expect("seeding fresh access-token expiry must succeed for this test to exercise the skip path");

    let resolver = resolver_with_refresh_and_store(accounts.clone(), secret_store);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[drive_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect("fresh OAuth token should be reused without refresh");

    // Must be the original handle — refresh was skipped.
    assert_eq!(resolved.handle, access_secret);
}

#[tokio::test]
async fn resolver_refreshes_when_access_token_is_within_margin() {
    // A2: An access secret with expires_at within the margin (or expired)
    // must trigger a refresh. The returned handle must be a new refreshed handle.
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let stale_access = SecretHandle::new("google_expiring_access").unwrap();
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    ConfiguredAccount::new(auth_scope, "google")
        .access_secret(Some(stale_access.clone()))
        .refresh_secret(SecretHandle::new("google_refresh_expiring").unwrap())
        .scopes(&["https://www.googleapis.com/auth/drive.readonly"])
        .create(&accounts)
        .await;

    // Pre-populate the secret store with an expiry within the margin (2 minutes from now).
    let secret_store = Arc::new(InMemorySecretStore::new());
    secret_store
        .put(
            scope.clone(),
            stale_access.clone(),
            ironclaw_secrets::SecretMaterial::from("[placeholder]".to_string()),
            Some(Utc::now() + chrono::Duration::minutes(2)),
        )
        .await
        .expect("seeding within-margin access-token expiry must succeed for this test to exercise the refresh path");

    let resolver = resolver_with_refresh_and_store(accounts.clone(), secret_store);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[drive_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect("within-margin OAuth token should trigger refresh");

    // Must be a new refreshed handle — refresh ran.
    assert_ne!(resolved.handle, stale_access);
    assert!(
        resolved
            .handle
            .as_str()
            .starts_with("oauth-refreshed-access")
    );
}
