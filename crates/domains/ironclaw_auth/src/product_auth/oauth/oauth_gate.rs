use std::fmt;
use std::sync::Arc;

use crate::{
    AuthChallenge, AuthContinuationRef, AuthEngine, AuthFlowId, AuthFlowKind, AuthFlowManager,
    AuthFlowOwnerScope, AuthFlowRecord, AuthFlowRecordSource, AuthGateRef, AuthProductError,
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel, NewAuthFlow,
    PrepareOAuthFlowRequest, ProviderScope, TurnGateAuthFlowQuery, TurnRunRef,
};
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_host_api::turn::{TurnRunId, TurnScope};
use ironclaw_host_api::{
    decision::RuntimeCredentialAuthRequirement,
    ids::{InvocationId, SecretHandle},
    resource::ResourceScope,
};
use ironclaw_secrets::{SecretMaterial, SecretStorePort};
use secrecy::SecretString;
use tokio::sync::Mutex as AsyncMutex;

const GATE_FLOW_TTL_SECONDS: i64 = 600;

#[derive(Clone, Copy)]
pub struct OAuthGateChallengeRequest<'a> {
    pub flow_manager: &'a Arc<dyn AuthFlowManager>,
    pub flow_source: &'a Arc<dyn AuthFlowRecordSource>,
    pub requirements: &'a [RuntimeCredentialAuthRequirement],
    pub scope: &'a TurnScope,
    pub owner_user_id: &'a ironclaw_host_api::ids::UserId,
    pub run_id: TurnRunId,
    pub gate_ref: &'a AuthGateRef,
}

/// Recipe-driven blocked-turn OAuth gate driver.
///
/// One driver for every vendor: the requirement's vendor id resolves to
/// recipe DATA through the engine's `AuthRecipeResolver`, and the engine
/// constructs the authorization URL/state/PKCE. The driver owns the shared
/// challenge/turn-gate-reuse/PKCE-store/cleanup/expiry logic — there is no
/// per-vendor gate provider and no vendor→driver registry.
#[derive(Clone)]
pub struct OAuthGateFlowDriver {
    engine: Arc<AuthEngine>,
    secret_store: Arc<dyn SecretStorePort>,
    setup_lock: Arc<AsyncMutex<()>>,
}

impl OAuthGateFlowDriver {
    pub fn new(engine: Arc<AuthEngine>, secret_store: Arc<dyn SecretStorePort>) -> Self {
        Self {
            engine,
            secret_store,
            setup_lock: Arc::new(AsyncMutex::new(())),
        }
    }

    pub async fn challenge_for_blocked_gate(
        &self,
        request: OAuthGateChallengeRequest<'_>,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        for requirement in request.requirements {
            let vendor = requirement.provider.as_str();
            if self
                .engine
                .recipes()
                .resolve(
                    Some(&requirement.requester_extension),
                    Some(request.owner_user_id),
                    vendor,
                )
                .await
                .is_none()
            {
                continue;
            }
            match self.challenge_for_requirement(request, requirement).await {
                Ok(challenge) => return Ok(Some(challenge)),
                // A resolvable-but-unconfigured vendor (e.g. missing operator
                // client credentials) must not swallow the whole gate: fall
                // through to the next requirement / the generic
                // requirement-derived prompt.
                Err(AuthProductError::BackendUnavailable | AuthProductError::MalformedConfig) => {
                    tracing::warn!(
                        vendor,
                        "OAuth gate vendor unavailable; falling through to next requirement"
                    );
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    fn pkce_secret_handle(&self, flow_id: AuthFlowId) -> Result<SecretHandle, AuthProductError> {
        SecretHandle::new(format!("oauth-gate-flow-pkce-{flow_id}")).map_err(|error| {
            tracing::warn!(
                flow_id = %flow_id,
                error = %error,
                "failed to build OAuth gate PKCE secret handle"
            );
            AuthProductError::BackendUnavailable
        })
    }

    async fn challenge_for_requirement(
        &self,
        request: OAuthGateChallengeRequest<'_>,
        requirement: &RuntimeCredentialAuthRequirement,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let auth_scope = auth_scope_for_blocked_turn(request.scope, request.owner_user_id);
        let turn_run_ref = TurnRunRef::new(request.run_id.to_string())?;
        let query = turn_gate_query(&auth_scope, request.scope, &turn_run_ref, request.gate_ref);
        let provider = AuthProviderId::new(requirement.provider.as_str())?;

        let _setup_guard = self.setup_lock.lock().await;
        if let Some(existing) = self
            .reusable_flow_for_query(
                request.flow_manager,
                request.flow_source,
                query.clone(),
                &provider,
                &requirement.requester_extension,
            )
            .await?
        {
            oauth_challenge_from_flow(&existing)?;
            return Ok(existing);
        }

        let flow_id = AuthFlowId::new();
        let vendor = requirement.provider.as_str();
        let prepared = self
            .engine
            .prepare_oauth_flow(PrepareOAuthFlowRequest {
                vendor: vendor.to_string(),
                requester_extension: Some(requirement.requester_extension.clone()),
                scope: auth_scope.clone(),
                flow_id,
                account_label: CredentialAccountLabel::new(vendor)?,
                requested_scopes: provider_scopes(&requirement.provider_scopes)?,
            })
            .await?;
        let expires_at = Utc::now() + ChronoDuration::seconds(GATE_FLOW_TTL_SECONDS);
        self.store_pkce_verifier(
            &auth_scope.resource,
            flow_id,
            prepared.pkce_verifier.clone(),
        )
        .await?;
        let flow = match request
            .flow_manager
            .create_flow(NewAuthFlow {
                requested_scopes: prepared.requested_scopes.clone(),
                id: Some(flow_id),
                scope: auth_scope.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: provider.clone(),
                requester_extension: prepared.requester_extension,
                challenge: AuthChallenge::OAuthUrl {
                    authorization_url: prepared.authorization_url,
                    expires_at,
                },
                continuation: AuthContinuationRef::TurnGateResume {
                    turn_run_ref,
                    gate_ref: request.gate_ref.clone(),
                },
                update_binding: None,
                opaque_state_hash: Some(prepared.opaque_state_hash),
                pkce_verifier_hash: Some(prepared.pkce_verifier_hash),
                expires_at,
            })
            .await
        {
            Ok(flow) => flow,
            Err(AuthProductError::BackendConflict) => {
                self.cleanup_pkce_verifier(&auth_scope.resource, flow_id)
                    .await;
                self.reusable_flow_for_query(
                    request.flow_manager,
                    request.flow_source,
                    query,
                    &provider,
                    &requirement.requester_extension,
                )
                .await?
                .ok_or(AuthProductError::BackendConflict)?
            }
            Err(error) => {
                self.cleanup_pkce_verifier(&auth_scope.resource, flow_id)
                    .await;
                return Err(error);
            }
        };

        oauth_challenge_from_flow(&flow)?;
        Ok(flow)
    }

    async fn reusable_flow_for_query(
        &self,
        flow_manager: &Arc<dyn AuthFlowManager>,
        flow_source: &Arc<dyn AuthFlowRecordSource>,
        query: TurnGateAuthFlowQuery,
        requested_provider: &AuthProviderId,
        requested_requester_extension: &ironclaw_host_api::ids::ExtensionId,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        let Some(existing) = flow_source.flow_for_turn_gate(query).await? else {
            return Ok(None);
        };
        if existing.provider != *requested_provider {
            self.cleanup_pkce_verifier(&existing.scope.resource, existing.id)
                .await;
            return flow_manager
                .cancel_flow(&existing.scope, existing.id)
                .await
                .map(|_| None);
        }
        if existing.expires_at > Utc::now() {
            // A live, unexpired flow whose requester differs from the
            // requirement asking to reuse it belongs to a different
            // extension's in-flight consent: decline reuse rather than
            // canceling it. Canceling would destroy another extension's
            // legitimate in-progress OAuth work for a mere
            // same-gate/same-vendor coincidence; the caller falls through to
            // creating its own flow under the same `setup_lock`/`gate_ref`,
            // so declining here is fail-closed, not racy. An expired flow
            // (below) carries no live consent to protect, so it is always
            // cleaned up regardless of requester.
            if existing.requester_extension.as_ref() != Some(requested_requester_extension) {
                return Ok(None);
            }
            return Ok(Some(existing));
        }
        // The flow being replaced is expired and about to be canceled; drop its
        // now-defunct PKCE verifier so it does not linger in the secret store.
        self.cleanup_pkce_verifier(&existing.scope.resource, existing.id)
            .await;
        flow_manager
            .cancel_flow(&existing.scope, existing.id)
            .await
            .map(|_| None)
    }

    async fn store_pkce_verifier(
        &self,
        scope: &ResourceScope,
        flow_id: AuthFlowId,
        material: SecretMaterial,
    ) -> Result<(), AuthProductError> {
        self.secret_store
            .put(
                scope.clone(),
                self.pkce_secret_handle(flow_id)?,
                material,
                None,
            )
            .await
            .map(|_| ())
            .map_err(|error| {
                tracing::warn!(
                    flow_id = %flow_id,
                    error = %error,
                    "failed to store OAuth gate PKCE verifier"
                );
                AuthProductError::BackendUnavailable
            })
    }

    async fn cleanup_pkce_verifier(&self, scope: &ResourceScope, flow_id: AuthFlowId) {
        let Ok(handle) = self.pkce_secret_handle(flow_id) else {
            return;
        };
        if self.secret_store.delete(scope, &handle).await.is_err() {
            tracing::warn!(
                flow_id = %flow_id,
                "failed to clean up OAuth gate PKCE verifier after flow creation failure"
            );
        }
    }

    pub async fn pkce_verifier_for_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<Option<SecretString>, AuthProductError> {
        let handle = self.pkce_secret_handle(flow_id)?;
        let lease = match self.secret_store.lease_once(&scope.resource, &handle).await {
            Ok(lease) => lease,
            Err(error) if error.is_unknown_secret() => return Ok(None),
            Err(error) => {
                tracing::warn!(
                    flow_id = %flow_id,
                    error = %error,
                    "failed to lease OAuth gate PKCE verifier"
                );
                return Err(AuthProductError::BackendUnavailable);
            }
        };
        self.secret_store
            .consume(&scope.resource, lease.id)
            .await
            .map(Some)
            .map_err(|error| {
                tracing::warn!(
                    flow_id = %flow_id,
                    error = %error,
                    "failed to consume OAuth gate PKCE verifier"
                );
                AuthProductError::BackendUnavailable
            })
    }
}

impl fmt::Debug for OAuthGateFlowDriver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthGateFlowDriver")
            .field("engine", &self.engine)
            .finish()
    }
}

pub fn auth_scope_for_blocked_turn(
    scope: &TurnScope,
    owner_user_id: &ironclaw_host_api::ids::UserId,
) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope {
            tenant_id: scope.tenant_id.clone(),
            user_id: owner_user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            mission_id: None,
            thread_id: Some(scope.thread_id.clone()),
            invocation_id: InvocationId::new(),
        },
        AuthSurface::Callback,
    )
}

pub fn turn_gate_query(
    auth_scope: &AuthProductScope,
    turn_scope: &TurnScope,
    turn_run_ref: &TurnRunRef,
    gate_ref: &AuthGateRef,
) -> TurnGateAuthFlowQuery {
    TurnGateAuthFlowQuery {
        owner: AuthFlowOwnerScope {
            tenant_id: auth_scope.resource.tenant_id.clone(),
            user_id: auth_scope.resource.user_id.clone(),
            agent_id: auth_scope.resource.agent_id.clone(),
            project_id: auth_scope.resource.project_id.clone(),
            thread_id: turn_scope.thread_id.clone(),
        },
        turn_run_ref: turn_run_ref.clone(),
        gate_ref: gate_ref.clone(),
        include_terminal: false,
    }
}

pub fn provider_scopes(raw_scopes: &[String]) -> Result<Vec<ProviderScope>, AuthProductError> {
    raw_scopes
        .iter()
        .map(|scope| ProviderScope::new(scope.clone()))
        .collect()
}

fn oauth_challenge_from_flow(flow: &AuthFlowRecord) -> Result<AuthChallenge, AuthProductError> {
    match flow.challenge.as_ref() {
        Some(AuthChallenge::OAuthUrl { .. }) => Ok(flow
            .challenge
            .clone()
            .ok_or(AuthProductError::BackendUnavailable)?),
        Some(_) | None => Err(AuthProductError::BackendUnavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthEngineDeps, AuthFlowStatus, EngineCallbackBase, InMemoryAuthProductServices,
        OAuthAuthorizationUrl, ResolvedVendorAuthRecipe, StaticAuthRecipeResolver,
    };
    use ironclaw_extension_contracts::recipe::VendorAuthRecipe;
    use ironclaw_host_api::ids::{AgentId, ExtensionId, TenantId, ThreadId, UserId, VendorId};
    use ironclaw_secrets::SecretStore;

    fn acme_vendor_recipe() -> ResolvedVendorAuthRecipe {
        let recipe: VendorAuthRecipe = serde_json::from_value(serde_json::json!({
            "method": "oauth2_code",
            "display_name": "Acme account",
            "authorization_endpoint": "https://auth.acme.example/authorize",
            "token_endpoint": "https://auth.acme.example/token",
            "scopes": ["msg:read"],
            "client_credentials": { "client_id_handle": "acme_oauth_client_id" },
            "token_response": { "access_token": "/access_token" },
        }))
        .expect("recipe parses");
        ResolvedVendorAuthRecipe {
            vendor: "acmevendor".to_string(),
            recipe,
            token_exchange_resource: None,
            protected_resource_metadata_url: None,
        }
    }

    #[derive(Debug)]
    struct StaticCredentials;

    #[async_trait::async_trait]
    impl crate::EngineClientCredentialsSource for StaticCredentials {
        async fn resolve(
            &self,
            _vendor: &str,
            _credentials: &ironclaw_extension_contracts::recipe::RecipeClientCredentials,
        ) -> Result<crate::EngineOAuthClientMaterial, AuthProductError> {
            Ok(crate::EngineOAuthClientMaterial {
                client_id: crate::OAuthClientId::new("gate-client-id")?,
                client_secret: None,
            })
        }
    }

    #[derive(Debug)]
    struct UnconfiguredCredentials;

    #[async_trait::async_trait]
    impl crate::EngineClientCredentialsSource for UnconfiguredCredentials {
        async fn resolve(
            &self,
            _vendor: &str,
            _credentials: &ironclaw_extension_contracts::recipe::RecipeClientCredentials,
        ) -> Result<crate::EngineOAuthClientMaterial, AuthProductError> {
            Err(AuthProductError::MalformedConfig)
        }
    }

    #[derive(Debug)]
    struct PanicEgress;

    #[async_trait::async_trait]
    impl ironclaw_host_api::http::RuntimeHttpEgress for PanicEgress {
        async fn execute(
            &self,
            _request: ironclaw_host_api::http::RuntimeHttpEgressRequest,
        ) -> Result<
            ironclaw_host_api::http::RuntimeHttpEgressResponse,
            ironclaw_host_api::http::RuntimeHttpEgressError,
        > {
            panic!("gate flow preparation must not reach the vendor");
        }
    }

    /// The unified recipe the production resolver builds once a SECOND
    /// extension declaring the same vendor is installed: identical recipe
    /// data, scope ceiling unioned across both manifests.
    fn acme_shared_vendor_recipe() -> ResolvedVendorAuthRecipe {
        let mut resolved = acme_vendor_recipe();
        let VendorAuthRecipe::Oauth2Code(recipe) = &mut resolved.recipe else {
            panic!("acme fixture is an oauth2_code recipe");
        };
        recipe.scopes.push("msg:write".to_string());
        resolved
    }

    /// The shape a vendor that declares no scopes resolves to: every
    /// hosted-MCP registration, whose dynamically registered vendor has no
    /// manifest to declare them, and the bundled `notion-mcp` manifest, which
    /// ships `scopes = []`.
    fn acme_empty_ceiling_recipe() -> ResolvedVendorAuthRecipe {
        let mut resolved = acme_vendor_recipe();
        let VendorAuthRecipe::Oauth2Code(recipe) = &mut resolved.recipe else {
            panic!("acme fixture is an oauth2_code recipe");
        };
        recipe.scopes.clear();
        resolved
    }

    fn engine_with_credentials(
        credentials: Arc<dyn crate::EngineClientCredentialsSource>,
    ) -> Arc<AuthEngine> {
        engine_with_recipe(acme_vendor_recipe(), credentials)
    }

    fn engine_with_recipe(
        recipe: ResolvedVendorAuthRecipe,
        credentials: Arc<dyn crate::EngineClientCredentialsSource>,
    ) -> Arc<AuthEngine> {
        Arc::new(AuthEngine::new(AuthEngineDeps {
            recipes: Arc::new(StaticAuthRecipeResolver::new(vec![recipe])),
            client_credentials: credentials,
            egress: Arc::new(PanicEgress),
            secret_store: Arc::new(SecretStore::ephemeral()),
            callback_base: EngineCallbackBase::new(
                "https://host.example/api/reborn/product-auth/oauth",
            )
            .expect("callback base"),
            dcr_client_name: "IronClaw test".to_string(),
        }))
    }

    struct GateFixture {
        shared: Arc<InMemoryAuthProductServices>,
        flow_manager: Arc<dyn AuthFlowManager>,
        flow_source: Arc<dyn AuthFlowRecordSource>,
        driver: OAuthGateFlowDriver,
        scope: TurnScope,
        owner_user_id: UserId,
        run_id: TurnRunId,
        gate_ref: AuthGateRef,
        requirement: RuntimeCredentialAuthRequirement,
    }

    impl GateFixture {
        fn new() -> Self {
            Self::with_recipe(acme_vendor_recipe())
        }

        fn with_recipe(recipe: ResolvedVendorAuthRecipe) -> Self {
            let shared = Arc::new(InMemoryAuthProductServices::new());
            let flow_manager: Arc<dyn AuthFlowManager> = shared.clone();
            let flow_source: Arc<dyn AuthFlowRecordSource> = shared.clone();
            Self {
                shared,
                flow_manager,
                flow_source,
                driver: OAuthGateFlowDriver::new(
                    engine_with_recipe(recipe, Arc::new(StaticCredentials)),
                    Arc::new(SecretStore::ephemeral()),
                ),
                scope: TurnScope::new(
                    TenantId::new("tenant-alpha").unwrap(),
                    Some(AgentId::new("agent-alpha").unwrap()),
                    None,
                    ThreadId::new("thread-alpha").unwrap(),
                ),
                owner_user_id: UserId::new("user-alpha").unwrap(),
                run_id: TurnRunId::new(),
                gate_ref: AuthGateRef::new("gate:vendor-auth").unwrap(),
                requirement: RuntimeCredentialAuthRequirement {
                    provider: VendorId::new("acmevendor").unwrap(),
                    setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
                        scopes: vec!["msg:read".to_string()],
                    },
                    requester_extension: ExtensionId::new("acme-messenger-fixture").unwrap(),
                    provider_scopes: vec!["msg:read".to_string()],
                },
            }
        }

        async fn challenge(&self) -> AuthFlowRecord {
            self.driver
                .challenge_for_blocked_gate(OAuthGateChallengeRequest {
                    flow_manager: &self.flow_manager,
                    flow_source: &self.flow_source,
                    requirements: std::slice::from_ref(&self.requirement),
                    scope: &self.scope,
                    owner_user_id: &self.owner_user_id,
                    run_id: self.run_id,
                    gate_ref: &self.gate_ref,
                })
                .await
                .unwrap()
                .expect("vendor requirement should produce a challenge")
        }

        fn auth_scope(&self) -> AuthProductScope {
            auth_scope_for_blocked_turn(&self.scope, &self.owner_user_id)
        }

        async fn active_gate_flows(&self) -> Vec<AuthFlowRecord> {
            let auth_scope = self.auth_scope();
            let turn_run_ref = TurnRunRef::new(self.run_id.to_string()).unwrap();
            let query = turn_gate_query(&auth_scope, &self.scope, &turn_run_ref, &self.gate_ref);
            self.shared
                .flows_for_owner(query.owner)
                .await
                .unwrap()
                .into_iter()
                .filter(|flow| {
                    flow.status == AuthFlowStatus::AwaitingUser
                        && matches!(
                            flow.continuation,
                            AuthContinuationRef::TurnGateResume { .. }
                        )
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn gate_challenge_is_recipe_driven_and_host_constructed() {
        let fixture = GateFixture::new();
        let flow = fixture.challenge().await;
        assert_eq!(flow.provider.as_str(), "acmevendor");
        assert_eq!(
            flow.requester_extension.as_ref(),
            Some(&fixture.requirement.requester_extension),
            "the gate flow durably retains the extension whose recipe was resolved"
        );
        let AuthChallenge::OAuthUrl {
            authorization_url: url,
            ..
        } = flow.challenge.expect("authorization challenge")
        else {
            panic!("expected OAuth URL challenge");
        };
        assert!(
            url.as_str()
                .starts_with("https://auth.acme.example/authorize")
        );
        assert!(url.as_str().contains("code_challenge"));
        assert_eq!(fixture.active_gate_flows().await.len(), 1);
    }

    /// The gate is the production caller that turns a blocked capability into
    /// an authorization URL. One vendor account is shared by every installed
    /// extension of that vendor, so the URL the gate builds must ask for the
    /// whole shared ceiling — otherwise connecting one extension leaves its
    /// siblings gated and they keep returning `auth_required` (#7069).
    #[tokio::test]
    async fn gate_challenge_requests_the_shared_vendor_ceiling() {
        let fixture = GateFixture::with_recipe(acme_shared_vendor_recipe());
        let flow = fixture.challenge().await;
        let AuthChallenge::OAuthUrl {
            authorization_url: url,
            ..
        } = flow.challenge.expect("authorization challenge")
        else {
            panic!("expected OAuth URL challenge");
        };
        let parsed = url::Url::parse(url.as_str()).expect("authorize URL is a valid URL");
        let scopes = parsed
            .query_pairs()
            .find(|(name, _)| name == "scope")
            .map(|(_, value)| value.into_owned())
            .expect("authorize URL carries a scope param");
        assert!(
            scopes.contains("msg:read"),
            "the blocked capability's own scope must still be requested; got {scopes}"
        );
        assert!(
            scopes.contains("msg:write"),
            "the sibling extension's scope must ride the same consent; got {scopes}"
        );
    }

    /// A vendor with no declared scopes must produce an authorize URL with no
    /// scope parameter at all, not one carrying `scope=`. RFC 6749 §3.3 makes
    /// the parameter optional but requires at least one token when present,
    /// and servers enforce that — Attio answers `scope=` with
    /// `400 invalid_scope` (#7308). The gate is the caller that reaches this
    /// first for a hosted-MCP vendor, since a blocked turn builds the URL
    /// before any Settings-initiated connect ever runs.
    #[tokio::test]
    async fn gate_challenge_omits_scope_param_for_empty_ceiling() {
        let mut fixture = GateFixture::with_recipe(acme_empty_ceiling_recipe());
        fixture.requirement.provider_scopes.clear();
        fixture.requirement.setup =
            ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
                scopes: Vec::new(),
            };
        let flow = fixture.challenge().await;
        let AuthChallenge::OAuthUrl {
            authorization_url: url,
            ..
        } = flow.challenge.expect("authorization challenge")
        else {
            panic!("expected OAuth URL challenge");
        };
        let parsed = url::Url::parse(url.as_str()).expect("authorize URL is a valid URL");
        assert!(
            !parsed.query_pairs().any(|(name, _)| name == "scope"),
            "an empty scope ceiling must omit the scope parameter, not send it \
             empty; got {parsed}"
        );
    }

    #[tokio::test]
    async fn gate_replaces_expired_turn_gate_flow() {
        let fixture = GateFixture::new();
        let expired_flow_id = AuthFlowId::new();
        let expired_scope = fixture.auth_scope();
        fixture
            .flow_manager
            .create_flow(NewAuthFlow {
                requested_scopes: Vec::new(),
                id: Some(expired_flow_id),
                scope: expired_scope.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: AuthProviderId::new("acmevendor").unwrap(),
                requester_extension: None,
                challenge: AuthChallenge::OAuthUrl {
                    authorization_url: OAuthAuthorizationUrl::new(
                        "https://auth.acme.example/authorize?state=expired".to_string(),
                    )
                    .unwrap(),
                    expires_at: Utc::now() - ChronoDuration::seconds(1),
                },
                continuation: AuthContinuationRef::TurnGateResume {
                    turn_run_ref: TurnRunRef::new(fixture.run_id.to_string()).unwrap(),
                    gate_ref: fixture.gate_ref.clone(),
                },
                update_binding: None,
                opaque_state_hash: None,
                pkce_verifier_hash: None,
                expires_at: Utc::now() - ChronoDuration::seconds(1),
            })
            .await
            .unwrap();

        let flow = fixture.challenge().await;
        let AuthChallenge::OAuthUrl {
            authorization_url, ..
        } = flow.challenge.expect("authorization challenge")
        else {
            panic!("expected OAuth URL challenge");
        };

        assert_ne!(
            authorization_url.as_str(),
            "https://auth.acme.example/authorize?state=expired"
        );
        let expired = fixture
            .shared
            .get_flow(&expired_scope, expired_flow_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(expired.status, AuthFlowStatus::Canceled);
        assert_eq!(fixture.active_gate_flows().await.len(), 1);
    }

    #[tokio::test]
    async fn gate_reuses_one_flow_under_concurrent_challenges() {
        let fixture = GateFixture::new();

        let (left, right) = tokio::join!(fixture.challenge(), fixture.challenge());
        let AuthChallenge::OAuthUrl {
            authorization_url: left,
            ..
        } = left.challenge.expect("left authorization challenge")
        else {
            panic!("expected left OAuth URL challenge");
        };
        let AuthChallenge::OAuthUrl {
            authorization_url: right,
            ..
        } = right.challenge.expect("right authorization challenge")
        else {
            panic!("expected right OAuth URL challenge");
        };

        assert_eq!(left, right);
        assert_eq!(fixture.active_gate_flows().await.len(), 1);
    }

    #[tokio::test]
    async fn gate_does_not_reuse_live_turn_gate_flow_for_different_provider() {
        let fixture = GateFixture::new();
        let auth_scope = fixture.auth_scope();
        let mismatched = fixture
            .flow_manager
            .create_flow(NewAuthFlow {
                requested_scopes: Vec::new(),
                id: Some(AuthFlowId::new()),
                scope: auth_scope,
                kind: AuthFlowKind::IntegrationCredential,
                provider: AuthProviderId::new("othervendor").unwrap(),
                requester_extension: None,
                challenge: AuthChallenge::OAuthUrl {
                    authorization_url: OAuthAuthorizationUrl::new(
                        "https://auth.other.example/authorize?state=existing".to_string(),
                    )
                    .unwrap(),
                    expires_at: Utc::now() + ChronoDuration::seconds(60),
                },
                continuation: AuthContinuationRef::TurnGateResume {
                    turn_run_ref: TurnRunRef::new(fixture.run_id.to_string()).unwrap(),
                    gate_ref: fixture.gate_ref.clone(),
                },
                update_binding: None,
                opaque_state_hash: None,
                pkce_verifier_hash: None,
                expires_at: Utc::now() + ChronoDuration::seconds(60),
            })
            .await
            .unwrap();

        let flow = fixture.challenge().await;
        assert_eq!(flow.provider.as_str(), "acmevendor");
        let AuthChallenge::OAuthUrl {
            authorization_url, ..
        } = flow.challenge.expect("authorization challenge")
        else {
            panic!("expected OAuth URL challenge");
        };
        assert!(
            authorization_url
                .as_str()
                .starts_with("https://auth.acme.example/authorize"),
            "same gate must not reuse another provider's authorization URL"
        );
        let mismatched = fixture
            .shared
            .get_flow(&mismatched.scope, mismatched.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            mismatched.status,
            AuthFlowStatus::Canceled,
            "same gate must cancel a stale live flow for another provider before replacement"
        );
        assert_eq!(fixture.active_gate_flows().await.len(), 1);
    }

    #[tokio::test]
    async fn gate_does_not_reuse_live_flow_for_a_different_requester_extension() {
        let fixture = GateFixture::new();
        let auth_scope = fixture.auth_scope();
        let other_extension_flow_id = AuthFlowId::new();
        let other_extension = fixture
            .flow_manager
            .create_flow(NewAuthFlow {
                requested_scopes: Vec::new(),
                id: Some(other_extension_flow_id),
                scope: auth_scope,
                kind: AuthFlowKind::IntegrationCredential,
                provider: AuthProviderId::new("acmevendor").unwrap(),
                requester_extension: Some(ExtensionId::new("other-extension-fixture").unwrap()),
                challenge: AuthChallenge::OAuthUrl {
                    authorization_url: OAuthAuthorizationUrl::new(
                        "https://auth.acme.example/authorize?state=other-extension".to_string(),
                    )
                    .unwrap(),
                    expires_at: Utc::now() + ChronoDuration::seconds(60),
                },
                continuation: AuthContinuationRef::TurnGateResume {
                    turn_run_ref: TurnRunRef::new(fixture.run_id.to_string()).unwrap(),
                    gate_ref: fixture.gate_ref.clone(),
                },
                update_binding: None,
                opaque_state_hash: None,
                pkce_verifier_hash: None,
                expires_at: Utc::now() + ChronoDuration::seconds(60),
            })
            .await
            .unwrap();

        // fixture.requirement's requester_extension is "acme-messenger-fixture",
        // distinct from the flow's "other-extension-fixture" requester, but
        // same owner/turn_run_ref/gate_ref/provider.
        let flow = fixture.challenge().await;

        assert_ne!(
            flow.id, other_extension.id,
            "a requirement from one extension must never receive another \
             extension's live, unexpired flow for the same vendor/gate"
        );
        let AuthChallenge::OAuthUrl {
            authorization_url, ..
        } = flow.challenge.expect("authorization challenge")
        else {
            panic!("expected OAuth URL challenge");
        };
        assert!(
            !authorization_url.as_str().contains("state=other-extension"),
            "same gate/vendor must not reuse another requester's authorization URL"
        );

        // Declining reuse is fail-closed, not destructive: the other
        // extension's still-valid flow must survive untouched.
        let other_extension = fixture
            .shared
            .get_flow(&other_extension.scope, other_extension.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            other_extension.status,
            AuthFlowStatus::AwaitingUser,
            "declining reuse must not cancel another extension's in-flight flow"
        );
    }

    /// A resolvable-but-unconfigured vendor (operator has not saved OAuth
    /// client credentials) must not swallow the whole gate: the driver falls
    /// through to the next requirement.
    #[tokio::test]
    async fn gate_falls_through_unconfigured_vendor_to_next_requirement() {
        let mut fixture = GateFixture::new();
        // A second engine whose credentials source rejects only the first
        // vendor would need two vendors; simpler: the first requirement names
        // a vendor with no recipe at all, the second is serviceable.
        let unknown_requirement = RuntimeCredentialAuthRequirement {
            provider: VendorId::new("unknownvendor").unwrap(),
            setup: Default::default(),
            requester_extension: ExtensionId::new("acme-messenger-fixture").unwrap(),
            provider_scopes: Vec::new(),
        };
        let serviceable = fixture.requirement.clone();
        fixture.requirement = unknown_requirement;
        let requirements = vec![fixture.requirement.clone(), serviceable];

        let challenge = fixture
            .driver
            .challenge_for_blocked_gate(OAuthGateChallengeRequest {
                flow_manager: &fixture.flow_manager,
                flow_source: &fixture.flow_source,
                requirements: &requirements,
                scope: &fixture.scope,
                owner_user_id: &fixture.owner_user_id,
                run_id: fixture.run_id,
                gate_ref: &fixture.gate_ref,
            })
            .await
            .unwrap()
            .expect("the serviceable requirement must still produce a challenge");
        assert_eq!(challenge.provider.as_str(), "acmevendor");

        // And an unconfigured (credentials missing) vendor also falls through.
        let unconfigured_driver = OAuthGateFlowDriver::new(
            engine_with_credentials(Arc::new(UnconfiguredCredentials)),
            Arc::new(SecretStore::ephemeral()),
        );
        let result = unconfigured_driver
            .challenge_for_blocked_gate(OAuthGateChallengeRequest {
                flow_manager: &fixture.flow_manager,
                flow_source: &fixture.flow_source,
                requirements: std::slice::from_ref(&requirements[1]),
                scope: &fixture.scope,
                owner_user_id: &fixture.owner_user_id,
                run_id: TurnRunId::new(),
                gate_ref: &fixture.gate_ref,
            })
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "unconfigured vendor yields no challenge instead of erroring the gate"
        );
    }
}
