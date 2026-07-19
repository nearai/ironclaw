use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowId, AuthFlowKind, AuthFlowManager,
    AuthFlowOwnerScope, AuthFlowRecord, AuthFlowRecordSource, AuthGateRef, AuthProductError,
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel, NewAuthFlow,
    OAuthCallbackState, OAuthCallbackStateKind, PkceVerifierSecret, ProviderScope, Timestamp,
    TurnGateAuthFlowQuery, TurnRunRef, build_google_authorization_url, opaque_state_hash,
    pkce_s256_challenge, pkce_verifier_hash,
};
use ironclaw_host_api::{
    InvocationId, ResourceScope, RuntimeCredentialAuthRequirement, SecretHandle,
};
use ironclaw_product_adapters::AuthPromptChallengeKind;
use ironclaw_secrets::{SecretMaterial, SecretStore};
use ironclaw_turns::{TurnRunId, TurnScope};
use secrecy::SecretString;
use tokio::sync::Mutex as AsyncMutex;

use crate::AuthChallengeView;
use crate::input::OAuthClientConfig;

const GATE_FLOW_TTL_SECONDS: i64 = 600;

/// Provider-specific pieces of a blocked-turn OAuth gate flow.
///
/// Everything else about the gate — challenge/turn-gate reuse, PKCE store &
/// cleanup, expiry replacement, conflict recovery, and challenge projection —
/// is provider-agnostic and lives in [`OAuthGateFlowDriver`]. Two production
/// implementors (Google, Slack personal) share that driver through this trait.
#[async_trait]
pub(crate) trait OAuthGateProvider: Send + Sync + fmt::Debug {
    /// Stable Reborn provider id (`google`, `slack_personal`, ...).
    fn provider_id(&self) -> &'static str;

    /// Prefix for the per-flow PKCE verifier secret handle. The driver appends
    /// `-{flow_id}` to build the full handle.
    fn pkce_secret_handle_label(&self) -> &'static str;

    /// Select the provider-owned flow that this blocked turn may reuse.
    ///
    /// Most providers accept the exact turn-gate candidate. Slack overrides
    /// this to validate the connection epoch and join its caller-wide flow.
    async fn select_reusable_flow(
        &self,
        _scope: &AuthProductScope,
        exact: Option<AuthFlowRecord>,
        _flow_source: &dyn AuthFlowRecordSource,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        Ok(exact)
    }

    /// Build the authorization URL + hashed state/PKCE material for a new flow.
    ///
    /// The only provider-specific step: Google emits `scope=` + offline-consent
    /// extras from static client config; Slack resolves client credentials from
    /// its setup slot and emits `user_scope=`.
    async fn prepare_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        scopes: Vec<ProviderScope>,
        expires_at: Timestamp,
    ) -> Result<PreparedOAuthGateFlow, AuthProductError>;

    /// Claim provider-owned lifecycle state only after the shared driver has
    /// durably published both the flow and its PKCE material. This ordering
    /// guarantees that a provider lifecycle epoch can always be joined by a
    /// competing process; a crash before this hook leaves only an adoptable
    /// flow, never an owner-wide lifecycle wedge.
    async fn publish_flow(
        &self,
        _scope: &AuthProductScope,
        _flow_id: AuthFlowId,
        _expires_at: Timestamp,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }

    /// Undo provider-owned state acquired by [`Self::prepare_flow`] when the
    /// shared gate driver cannot publish or later retires the auth flow.
    async fn abandon_flow(&self, _scope: &AuthProductScope, _flow_id: AuthFlowId) {}
}

/// One generic registry over every OAuth gate provider (Google, Slack personal).
///
/// Replaces the former parallel per-provider registries: a single
/// `Option<Arc<OAuthGateProviderRegistry>>` slot on the product-auth bundle and
/// one dispatch arm route every provider's blocked-gate challenge and PKCE
/// lookup through the shared [`OAuthGateFlowDriver`].
#[derive(Clone)]
pub(crate) struct OAuthGateProviderRegistry {
    drivers: BTreeMap<String, Arc<OAuthGateFlowDriver>>,
}

impl OAuthGateProviderRegistry {
    pub(crate) fn new(drivers: Vec<Arc<OAuthGateFlowDriver>>) -> Self {
        let mut map: BTreeMap<String, Arc<OAuthGateFlowDriver>> = BTreeMap::new();
        for driver in drivers {
            if let Some(previous) = map.insert(driver.provider_id().to_string(), driver) {
                tracing::warn!(
                    provider = previous.provider_id(),
                    "duplicate OAuth gate provider registered; last registration wins"
                );
            }
        }
        Self { drivers: map }
    }

    pub(crate) async fn challenge_for_blocked_gate(
        &self,
        request: OAuthGateChallengeRequest<'_>,
    ) -> Result<Option<AuthChallengeView>, AuthProductError> {
        for requirement in request.requirements {
            let Some(driver) = self.drivers.get(requirement.provider.as_str()) else {
                continue;
            };
            match driver
                .challenge_for_blocked_gate(request, requirement)
                .await
            {
                Ok(challenge) => return Ok(Some(challenge)),
                // A registered-but-unconfigured provider (e.g. the Slack
                // personal slot before the operator saves OAuth client
                // credentials) must not swallow the whole gate: fall through to
                // the next requirement / the generic requirement-derived
                // prompt, matching the pre-registry behavior where an
                // un-serviceable requirement was simply skipped.
                Err(AuthProductError::BackendUnavailable) => {
                    tracing::warn!(
                        provider = requirement.provider.as_str(),
                        "OAuth gate provider unavailable; falling through to next requirement"
                    );
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    pub(crate) async fn pkce_verifier_for_flow(
        &self,
        scope: &AuthProductScope,
        provider: &AuthProviderId,
        flow_id: AuthFlowId,
    ) -> Result<Option<SecretString>, AuthProductError> {
        let Some(driver) = self.drivers.get(provider.as_str()) else {
            return Ok(None);
        };
        driver.pkce_verifier_for_flow(scope, flow_id).await
    }
}

impl fmt::Debug for OAuthGateProviderRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthGateProviderRegistry")
            .field("providers", &self.drivers.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Clone, Copy)]
pub(crate) struct OAuthGateChallengeRequest<'a> {
    pub(crate) flow_manager: &'a Arc<dyn AuthFlowManager>,
    pub(crate) flow_source: &'a Arc<dyn AuthFlowRecordSource>,
    pub(crate) requirements: &'a [RuntimeCredentialAuthRequirement],
    pub(crate) scope: &'a TurnScope,
    pub(crate) owner_user_id: &'a ironclaw_host_api::UserId,
    pub(crate) run_id: TurnRunId,
    pub(crate) gate_ref: &'a AuthGateRef,
}

/// Provider-agnostic blocked-turn OAuth gate driver.
///
/// Holds the shared challenge/turn-gate-reuse/PKCE-store/cleanup/expiry logic
/// that was previously duplicated between the Google and Slack gate providers.
/// Delegates only the per-provider flow preparation to an [`OAuthGateProvider`].
#[derive(Clone)]
pub(crate) struct OAuthGateFlowDriver {
    provider: Arc<dyn OAuthGateProvider>,
    secret_store: Arc<dyn SecretStore>,
    setup_lock: Arc<AsyncMutex<()>>,
}

impl OAuthGateFlowDriver {
    pub(crate) fn new(
        provider: Arc<dyn OAuthGateProvider>,
        secret_store: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            provider,
            secret_store,
            setup_lock: Arc::new(AsyncMutex::new(())),
        }
    }

    fn provider_id(&self) -> &'static str {
        self.provider.provider_id()
    }

    fn pkce_secret_handle(&self, flow_id: AuthFlowId) -> Result<SecretHandle, AuthProductError> {
        SecretHandle::new(format!(
            "{}-{flow_id}",
            self.provider.pkce_secret_handle_label()
        ))
        .map_err(|error| {
            tracing::warn!(
                provider = self.provider_id(),
                flow_id = %flow_id,
                error = %error,
                "failed to build OAuth gate PKCE secret handle"
            );
            AuthProductError::BackendUnavailable
        })
    }

    async fn challenge_for_blocked_gate(
        &self,
        request: OAuthGateChallengeRequest<'_>,
        requirement: &RuntimeCredentialAuthRequirement,
    ) -> Result<AuthChallengeView, AuthProductError> {
        let auth_scope = auth_scope_for_blocked_turn(request.scope, request.owner_user_id);
        let turn_run_ref = TurnRunRef::new(request.run_id.to_string())?;
        let query = turn_gate_query(&auth_scope, request.scope, &turn_run_ref, request.gate_ref);

        let _setup_guard = self.setup_lock.lock().await;
        let exact = self
            .reusable_flow_for_query(request.flow_manager, request.flow_source, query.clone())
            .await?;
        if let Some(existing) = self
            .provider
            .select_reusable_flow(&auth_scope, exact.clone(), request.flow_source.as_ref())
            .await?
        {
            if let Some(rejected_exact) = exact.filter(|flow| flow.id != existing.id) {
                self.retire_flow(request.flow_manager, rejected_exact).await;
            }
            return challenge_view_from_flow(&existing);
        }
        if let Some(rejected_exact) = exact {
            self.retire_flow(request.flow_manager, rejected_exact).await;
        }

        let flow_id = AuthFlowId::new();
        let scopes = provider_scopes(&requirement.provider_scopes)?;
        let expires_at = Utc::now() + ChronoDuration::seconds(GATE_FLOW_TTL_SECONDS);
        let prepared = self
            .provider
            .prepare_flow(&auth_scope, flow_id, scopes, expires_at)
            .await?;
        if let Err(error) = self
            .store_pkce_verifier(
                &auth_scope.resource,
                flow_id,
                prepared.pkce_verifier.clone(),
            )
            .await
        {
            self.provider.abandon_flow(&auth_scope, flow_id).await;
            return Err(error);
        }
        let flow = match request
            .flow_manager
            .create_flow(NewAuthFlow {
                id: Some(flow_id),
                scope: auth_scope.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: AuthProviderId::new(self.provider_id())?,
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
                self.provider.abandon_flow(&auth_scope, flow_id).await;
                let existing = self
                    .reusable_flow_for_query(request.flow_manager, request.flow_source, query)
                    .await?
                    .ok_or(AuthProductError::BackendConflict)?;
                self.provider
                    .publish_flow(&auth_scope, existing.id, existing.expires_at)
                    .await?;
                self.provider
                    .select_reusable_flow(&auth_scope, Some(existing), request.flow_source.as_ref())
                    .await?
                    .ok_or(AuthProductError::BackendConflict)?
            }
            Err(error) => {
                self.cleanup_pkce_verifier(&auth_scope.resource, flow_id)
                    .await;
                self.provider.abandon_flow(&auth_scope, flow_id).await;
                return Err(error);
            }
        };

        if let Err(error) = self
            .provider
            .publish_flow(&auth_scope, flow.id, flow.expires_at)
            .await
        {
            self.cleanup_pkce_verifier(&auth_scope.resource, flow.id)
                .await;
            let _ = request.flow_manager.cancel_flow(&flow.scope, flow.id).await;
            self.provider.abandon_flow(&auth_scope, flow.id).await;
            if error == AuthProductError::BackendConflict
                && let Some(existing) = self
                    .provider
                    .select_reusable_flow(&auth_scope, None, request.flow_source.as_ref())
                    .await?
            {
                return challenge_view_from_flow(&existing);
            }
            return Err(error);
        }

        match challenge_view_from_flow(&flow) {
            Ok(challenge) => Ok(challenge),
            Err(error) => {
                self.cleanup_pkce_verifier(&auth_scope.resource, flow_id)
                    .await;
                self.provider.abandon_flow(&auth_scope, flow_id).await;
                let _ = request.flow_manager.cancel_flow(&auth_scope, flow_id).await;
                Err(error)
            }
        }
    }

    async fn reusable_flow_for_query(
        &self,
        flow_manager: &Arc<dyn AuthFlowManager>,
        flow_source: &Arc<dyn AuthFlowRecordSource>,
        query: TurnGateAuthFlowQuery,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        let Some(existing) = flow_source.flow_for_turn_gate(query).await? else {
            return Ok(None);
        };
        if existing.expires_at > Utc::now() {
            return Ok(Some(existing));
        }
        // The flow being replaced is expired and about to be canceled; drop its
        // now-defunct PKCE verifier so it does not linger in the secret store.
        self.cleanup_pkce_verifier(&existing.scope.resource, existing.id)
            .await;
        let canceled = flow_manager
            .cancel_flow(&existing.scope, existing.id)
            .await?;
        self.provider
            .abandon_flow(&canceled.scope, canceled.id)
            .await;
        Ok(None)
    }

    async fn retire_flow(&self, flow_manager: &Arc<dyn AuthFlowManager>, flow: AuthFlowRecord) {
        self.cleanup_pkce_verifier(&flow.scope.resource, flow.id)
            .await;
        let _ = flow_manager.cancel_flow(&flow.scope, flow.id).await;
        self.provider.abandon_flow(&flow.scope, flow.id).await;
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
                    provider = self.provider_id(),
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
                provider = self.provider_id(),
                flow_id = %flow_id,
                "failed to clean up OAuth gate PKCE verifier after flow creation failure"
            );
        }
    }

    async fn pkce_verifier_for_flow(
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
                    provider = self.provider_id(),
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
                    provider = self.provider_id(),
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
            .field("provider", &self.provider)
            .finish()
    }
}

/// Google product-auth blocked-turn OAuth gate provider.
///
/// Holds static Google OAuth client config; the shared [`OAuthGateFlowDriver`]
/// owns everything else.
#[derive(Clone)]
pub(crate) struct GoogleOAuthGateProvider {
    client: OAuthClientConfig,
}

impl GoogleOAuthGateProvider {
    pub(crate) fn new(client: OAuthClientConfig) -> Self {
        Self { client }
    }
}

#[async_trait]
impl OAuthGateProvider for GoogleOAuthGateProvider {
    fn provider_id(&self) -> &'static str {
        ironclaw_auth::GOOGLE_PROVIDER_ID
    }

    fn pkce_secret_handle_label(&self) -> &'static str {
        "google-oauth-gate-flow-pkce"
    }

    async fn prepare_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        scopes: Vec<ProviderScope>,
        _expires_at: Timestamp,
    ) -> Result<PreparedOAuthGateFlow, AuthProductError> {
        let account_label = CredentialAccountLabel::new("google")?;
        let state = OAuthCallbackState::new(
            OAuthCallbackStateKind::GOOGLE,
            flow_id,
            scope.clone(),
            account_label,
            scopes.clone(),
        )?
        .encode()?;
        let opaque_state_hash = opaque_state_hash(state.as_str())?;
        let pkce_verifier = SecretString::from(ironclaw_common::pkce::generate_code_verifier());
        let pkce_secret = PkceVerifierSecret::new(pkce_verifier.clone())?;
        let pkce_verifier_hash = pkce_verifier_hash(&pkce_secret)?;
        let pkce_challenge = pkce_s256_challenge(&pkce_secret);
        let authorization_url = build_google_authorization_url(
            self.client.client_id.as_str(),
            self.client.redirect_uri.as_str(),
            state.as_str(),
            &pkce_challenge,
            &scopes,
            self.client.hosted_domain_hint.as_deref(),
        )?;
        Ok(PreparedOAuthGateFlow {
            authorization_url,
            opaque_state_hash,
            pkce_verifier_hash,
            pkce_verifier,
        })
    }
}

impl fmt::Debug for GoogleOAuthGateProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleOAuthGateProvider")
            .field("client_id", &self.client.client_id.as_str())
            .field("redirect_uri", &self.client.redirect_uri)
            .finish()
    }
}

/// Authorization URL + hashed state/PKCE material for a new gate flow, produced
/// by [`OAuthGateProvider::prepare_flow`] and consumed by the shared driver.
#[derive(Debug)]
pub(crate) struct PreparedOAuthGateFlow {
    pub(crate) authorization_url: ironclaw_auth::OAuthAuthorizationUrl,
    pub(crate) opaque_state_hash: ironclaw_auth::OpaqueStateHash,
    pub(crate) pkce_verifier_hash: ironclaw_auth::PkceVerifierHash,
    pub(crate) pkce_verifier: SecretString,
}

pub(crate) fn auth_scope_for_blocked_turn(
    scope: &TurnScope,
    owner_user_id: &ironclaw_host_api::UserId,
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

pub(crate) fn turn_gate_query(
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

pub(crate) fn provider_scopes(
    raw_scopes: &[String],
) -> Result<Vec<ProviderScope>, AuthProductError> {
    raw_scopes
        .iter()
        .map(|scope| ProviderScope::new(scope.clone()))
        .collect()
}

pub(crate) fn challenge_view_from_flow(
    flow: &AuthFlowRecord,
) -> Result<AuthChallengeView, AuthProductError> {
    match flow.challenge.as_ref() {
        Some(AuthChallenge::OAuthUrl {
            authorization_url,
            expires_at,
        }) => Ok(AuthChallengeView {
            kind: AuthPromptChallengeKind::OAuthUrl,
            provider: flow.provider.clone(),
            account_label: None,
            authorization_url: Some(authorization_url.clone()),
            expires_at: Some(*expires_at),
        }),
        Some(_) | None => Err(AuthProductError::BackendUnavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    use ironclaw_auth::{
        AuthFlowStatus, GOOGLE_CALENDAR_READONLY_SCOPE, InMemoryAuthProductServices,
        OAuthAuthorizationUrl,
    };
    use ironclaw_host_api::{
        AgentId, ExtensionId, RuntimeCredentialAccountProviderId, TenantId, ThreadId, UserId,
    };
    use ironclaw_secrets::FilesystemSecretStore;
    use tokio::sync::Notify;
    use tokio::time::timeout;

    #[tokio::test]
    async fn google_oauth_gate_replaces_expired_turn_gate_flow() {
        let fixture = GateFixture::new(None);
        let expired_flow_id = AuthFlowId::new();
        let expired_scope = fixture.auth_scope();
        fixture
            .flow_manager
            .create_flow(NewAuthFlow {
                id: Some(expired_flow_id),
                scope: expired_scope.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: AuthProviderId::new(ironclaw_auth::GOOGLE_PROVIDER_ID).unwrap(),
                challenge: AuthChallenge::OAuthUrl {
                    authorization_url: OAuthAuthorizationUrl::new(
                        "https://accounts.google.com/o/oauth2/v2/auth?state=expired".to_string(),
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

        let challenge = fixture.challenge().await;

        assert_ne!(
            challenge.authorization_url.unwrap().as_str(),
            "https://accounts.google.com/o/oauth2/v2/auth?state=expired"
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
    async fn google_oauth_gate_reuses_one_flow_under_concurrent_challenges() {
        let fixture = GateFixture::new(None);

        let (left, right) = tokio::join!(fixture.challenge(), fixture.challenge());
        let left = left.authorization_url.unwrap();
        let right = right.authorization_url.unwrap();

        assert_eq!(left, right);
        assert_eq!(fixture.active_gate_flows().await.len(), 1);
    }

    #[tokio::test]
    async fn owner_flow_reuse_converges_when_first_publisher_is_delayed() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let state = Arc::new(RaceOAuthProviderState::default());
        state.block_first_publish.store(true, Ordering::SeqCst);
        let provider = Arc::new(RaceOAuthProvider {
            state: state.clone(),
        });
        let secret_store = Arc::new(FilesystemSecretStore::ephemeral());
        let winner_driver = Arc::new(OAuthGateFlowDriver::new(
            provider.clone(),
            secret_store.clone(),
        ));
        let loser_driver = Arc::new(OAuthGateFlowDriver::new(provider, secret_store));
        let owner_user_id = UserId::new("user-race").unwrap();
        let winner_scope = race_scope("thread-race-winner");
        let loser_scope = race_scope("thread-race-loser");
        let winner_run_id = TurnRunId::new();
        let loser_run_id = TurnRunId::new();
        let winner_gate_ref = AuthGateRef::new("gate:race-winner").unwrap();
        let loser_gate_ref = AuthGateRef::new("gate:race-loser").unwrap();
        let requirement = race_requirement();

        let winner = tokio::spawn(race_challenge(
            winner_driver,
            shared.clone(),
            winner_scope.clone(),
            owner_user_id.clone(),
            winner_run_id,
            winner_gate_ref.clone(),
            requirement.clone(),
        ));
        timeout(
            Duration::from_secs(2),
            state.first_publish_entered.notified(),
        )
        .await
        .expect("first publisher must reach the post-publication claim hook");

        // This is intentionally longer than the deleted 40 ms polling window:
        // the loser no longer waits for an unpublished lifecycle winner.
        tokio::time::sleep(Duration::from_millis(75)).await;

        let loser = tokio::spawn(race_challenge(
            loser_driver,
            shared.clone(),
            loser_scope.clone(),
            owner_user_id.clone(),
            loser_run_id,
            loser_gate_ref.clone(),
            requirement,
        ));
        let loser_challenge = timeout(Duration::from_secs(2), loser)
            .await
            .expect("second publisher challenge timed out")
            .expect("second publisher task")
            .expect("second publisher challenge");
        state.release_first_publish.notify_one();
        let winner_challenge = timeout(Duration::from_secs(2), winner)
            .await
            .expect("delayed publisher challenge timed out")
            .expect("delayed publisher task")
            .expect("delayed publisher challenge");

        assert_eq!(
            winner_challenge.authorization_url, loser_challenge.authorization_url,
            "both processes must converge on the lifecycle winner's published flow"
        );
        assert_eq!(state.prepare_calls.load(Ordering::SeqCst), 2);
        let active = shared
            .flow_records_snapshot()
            .into_iter()
            .filter(|flow| flow.status == AuthFlowStatus::AwaitingUser)
            .collect::<Vec<_>>();
        assert_eq!(active.len(), 1, "the race must publish one auth flow");
        assert_eq!(active[0].id, state.current_epoch().unwrap());
        assert_eq!(
            active[0].continuation,
            AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(loser_run_id.to_string()).unwrap(),
                gate_ref: loser_gate_ref,
            }
        );
    }

    #[tokio::test]
    async fn retry_adopts_flow_when_publisher_dies_before_lifecycle_claim() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let state = Arc::new(RaceOAuthProviderState::default());
        state.block_first_publish.store(true, Ordering::SeqCst);
        let provider = Arc::new(RaceOAuthProvider {
            state: state.clone(),
        });
        let secret_store = Arc::new(FilesystemSecretStore::ephemeral());
        let crashed_driver = Arc::new(OAuthGateFlowDriver::new(
            provider.clone(),
            secret_store.clone(),
        ));
        let retry_driver = Arc::new(OAuthGateFlowDriver::new(provider, secret_store));
        let owner_user_id = UserId::new("user-race").unwrap();
        let scope = race_scope("thread-publisher-crash");
        let run_id = TurnRunId::new();
        let gate_ref = AuthGateRef::new("gate:publisher-crash").unwrap();
        let requirement = race_requirement();

        let crashed = tokio::spawn(race_challenge(
            crashed_driver,
            shared.clone(),
            scope.clone(),
            owner_user_id.clone(),
            run_id,
            gate_ref.clone(),
            requirement.clone(),
        ));
        timeout(
            Duration::from_secs(2),
            state.first_publish_entered.notified(),
        )
        .await
        .expect("flow must be durable before the claim hook blocks");
        crashed.abort();
        let _ = crashed.await;
        assert!(
            state.current_epoch().is_none(),
            "crashed publisher never claimed lifecycle"
        );

        let challenge = race_challenge(
            retry_driver,
            shared.clone(),
            scope,
            owner_user_id,
            run_id,
            gate_ref,
            requirement,
        )
        .await
        .expect("retry adopts the exact durable flow");
        let active = shared
            .flow_records_snapshot()
            .into_iter()
            .filter(|flow| flow.status == AuthFlowStatus::AwaitingUser)
            .collect::<Vec<_>>();
        assert_eq!(active.len(), 1);
        assert_eq!(state.current_epoch(), Some(active[0].id));
        assert_eq!(
            challenge.authorization_url,
            challenge_view_from_flow(&active[0])
                .unwrap()
                .authorization_url
        );
    }

    #[tokio::test]
    async fn google_oauth_gate_authorization_url_keeps_hosted_domain_hint() {
        let fixture = GateFixture::new(Some("example.com"));

        let challenge = fixture.challenge().await;
        let authorization_url = challenge.authorization_url.unwrap();
        let parsed = url::Url::parse(authorization_url.as_str()).unwrap();

        assert!(
            parsed
                .query_pairs()
                .any(|(name, value)| name == "hd" && value == "example.com")
        );
    }

    #[tokio::test]
    async fn google_oauth_gate_registry_uses_registered_requirement_when_multiple_present() {
        let fixture = GateFixture::new(None);
        let registry = OAuthGateProviderRegistry::new(vec![Arc::new(fixture.driver.clone())]);
        let unsupported_requirement = RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: Default::default(),
            requester_extension: ExtensionId::new("github").unwrap(),
            provider_scopes: Vec::new(),
        };
        let requirements = vec![unsupported_requirement, fixture.requirement.clone()];

        let challenge = registry
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
            .expect("google requirement should produce a challenge");

        assert_eq!(challenge.kind, AuthPromptChallengeKind::OAuthUrl);
        assert_eq!(fixture.active_gate_flows().await.len(), 1);
    }

    /// A registered-but-unconfigured provider (Slack before the operator saves
    /// OAuth client credentials) must not swallow the whole gate: pre-fix, a
    /// requirements list with `slack_personal` first errored `challenge_for_gate`
    /// entirely and the user got no auth prompt at all.
    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn gate_registry_falls_through_unavailable_provider_to_next_requirement() {
        use crate::slack::slack_personal_oauth::SlackPersonalOAuthGateProvider;
        use crate::slack::slack_setup::SlackPersonalSetupServiceSlot;

        let fixture = GateFixture::new(None);
        let slack_driver = Arc::new(OAuthGateFlowDriver::new(
            Arc::new(SlackPersonalOAuthGateProvider::new(
                SlackPersonalSetupServiceSlot::new(
                    ironclaw_auth::OAuthRedirectUri::new(
                        "https://host.example/api/reborn/product-auth/oauth/slack_personal/callback",
                    )
                    .unwrap(),
                ),
            )),
            Arc::new(FilesystemSecretStore::ephemeral()),
        ));
        let registry =
            OAuthGateProviderRegistry::new(vec![slack_driver, Arc::new(fixture.driver.clone())]);
        let slack_requirement = RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("slack_personal").unwrap(),
            setup: Default::default(),
            requester_extension: ExtensionId::new("slack").unwrap(),
            provider_scopes: Vec::new(),
        };
        let requirements = vec![slack_requirement, fixture.requirement.clone()];

        let challenge = registry
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
            .expect("the google requirement must still produce a challenge");

        assert_eq!(
            challenge.provider.as_str(),
            ironclaw_auth::GOOGLE_PROVIDER_ID
        );
    }

    #[derive(Default)]
    struct RaceOAuthProviderState {
        current_epoch: Mutex<Option<AuthFlowId>>,
        block_first_publish: AtomicBool,
        prepare_calls: AtomicUsize,
        publish_calls: AtomicUsize,
        first_publish_entered: Notify,
        release_first_publish: Notify,
    }

    impl RaceOAuthProviderState {
        fn current_epoch(&self) -> Option<AuthFlowId> {
            *self.current_epoch.lock().expect("race epoch lock")
        }
    }

    struct RaceOAuthProvider {
        state: Arc<RaceOAuthProviderState>,
    }

    impl fmt::Debug for RaceOAuthProvider {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("RaceOAuthProvider")
                .finish_non_exhaustive()
        }
    }

    #[async_trait]
    impl OAuthGateProvider for RaceOAuthProvider {
        fn provider_id(&self) -> &'static str {
            "race_oauth"
        }

        fn pkce_secret_handle_label(&self) -> &'static str {
            "race-oauth-pkce"
        }

        async fn select_reusable_flow(
            &self,
            scope: &AuthProductScope,
            exact: Option<AuthFlowRecord>,
            flow_source: &dyn AuthFlowRecordSource,
        ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
            if let Some(epoch) = self.state.current_epoch() {
                return flow_source.flow_for_owner_by_id(scope, epoch).await;
            }
            if let Some(exact) = exact {
                let mut epoch = self.state.current_epoch.lock().expect("race epoch lock");
                if epoch.is_none() {
                    *epoch = Some(exact.id);
                }
                return Ok((*epoch == Some(exact.id)).then_some(exact));
            }
            Ok(None)
        }

        async fn prepare_flow(
            &self,
            _scope: &AuthProductScope,
            flow_id: AuthFlowId,
            _scopes: Vec<ProviderScope>,
            _expires_at: Timestamp,
        ) -> Result<PreparedOAuthGateFlow, AuthProductError> {
            self.state.prepare_calls.fetch_add(1, Ordering::SeqCst);
            Ok(race_prepared_flow(flow_id))
        }

        async fn publish_flow(
            &self,
            _scope: &AuthProductScope,
            flow_id: AuthFlowId,
            _expires_at: Timestamp,
        ) -> Result<(), AuthProductError> {
            let publish_call = self.state.publish_calls.fetch_add(1, Ordering::SeqCst);
            if publish_call == 0 && self.state.block_first_publish.load(Ordering::SeqCst) {
                self.state.first_publish_entered.notify_one();
                self.state.release_first_publish.notified().await;
            }
            let mut epoch = self.state.current_epoch.lock().expect("race epoch lock");
            match *epoch {
                Some(current) if current == flow_id => Ok(()),
                Some(_) => Err(AuthProductError::BackendConflict),
                None => {
                    *epoch = Some(flow_id);
                    Ok(())
                }
            }
        }
    }

    fn race_prepared_flow(flow_id: AuthFlowId) -> PreparedOAuthGateFlow {
        let pkce_verifier = SecretString::from(ironclaw_common::pkce::generate_code_verifier());
        let pkce_secret = PkceVerifierSecret::new(pkce_verifier.clone()).unwrap();
        PreparedOAuthGateFlow {
            authorization_url: OAuthAuthorizationUrl::new(format!(
                "https://provider.example/oauth?flow_id={flow_id}"
            ))
            .unwrap(),
            opaque_state_hash: opaque_state_hash(format!("state-{flow_id}").as_str()).unwrap(),
            pkce_verifier_hash: pkce_verifier_hash(&pkce_secret).unwrap(),
            pkce_verifier,
        }
    }

    fn race_scope(thread_id: &str) -> TurnScope {
        TurnScope::new(
            TenantId::new("tenant-race").unwrap(),
            Some(AgentId::new("agent-race").unwrap()),
            None,
            ThreadId::new(thread_id).unwrap(),
        )
    }

    fn race_requirement() -> RuntimeCredentialAuthRequirement {
        RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("race_oauth").unwrap(),
            setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            requester_extension: ExtensionId::new("race-extension").unwrap(),
            provider_scopes: Vec::new(),
        }
    }

    async fn race_challenge(
        driver: Arc<OAuthGateFlowDriver>,
        shared: Arc<InMemoryAuthProductServices>,
        scope: TurnScope,
        owner_user_id: UserId,
        run_id: TurnRunId,
        gate_ref: AuthGateRef,
        requirement: RuntimeCredentialAuthRequirement,
    ) -> Result<AuthChallengeView, AuthProductError> {
        let flow_manager: Arc<dyn AuthFlowManager> = shared.clone();
        let flow_source: Arc<dyn AuthFlowRecordSource> = shared;
        driver
            .challenge_for_blocked_gate(
                OAuthGateChallengeRequest {
                    flow_manager: &flow_manager,
                    flow_source: &flow_source,
                    requirements: std::slice::from_ref(&requirement),
                    scope: &scope,
                    owner_user_id: &owner_user_id,
                    run_id,
                    gate_ref: &gate_ref,
                },
                &requirement,
            )
            .await
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
        fn new(hosted_domain_hint: Option<&str>) -> Self {
            let shared = Arc::new(InMemoryAuthProductServices::new());
            let flow_manager: Arc<dyn AuthFlowManager> = shared.clone();
            let flow_source: Arc<dyn AuthFlowRecordSource> = shared.clone();
            let mut client = OAuthClientConfig::new(
                "google-client.apps.googleusercontent.com",
                "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
                None,
            )
            .unwrap();
            if let Some(hosted_domain_hint) = hosted_domain_hint {
                client = client.with_hosted_domain_hint(hosted_domain_hint);
            }
            Self {
                shared,
                flow_manager,
                flow_source,
                driver: OAuthGateFlowDriver::new(
                    Arc::new(GoogleOAuthGateProvider::new(client)),
                    Arc::new(FilesystemSecretStore::ephemeral()),
                ),
                scope: TurnScope::new(
                    TenantId::new("tenant-alpha").unwrap(),
                    Some(AgentId::new("agent-alpha").unwrap()),
                    None,
                    ThreadId::new("thread-alpha").unwrap(),
                ),
                owner_user_id: UserId::new("user-alpha").unwrap(),
                run_id: TurnRunId::new(),
                gate_ref: AuthGateRef::new("gate:google-auth").unwrap(),
                requirement: RuntimeCredentialAuthRequirement {
                    provider: RuntimeCredentialAccountProviderId::new("google").unwrap(),
                    setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
                        scopes: vec![GOOGLE_CALENDAR_READONLY_SCOPE.to_string()],
                    },
                    requester_extension: ExtensionId::new("google-calendar").unwrap(),
                    provider_scopes: vec![GOOGLE_CALENDAR_READONLY_SCOPE.to_string()],
                },
            }
        }

        async fn challenge(&self) -> AuthChallengeView {
            self.driver
                .challenge_for_blocked_gate(
                    OAuthGateChallengeRequest {
                        flow_manager: &self.flow_manager,
                        flow_source: &self.flow_source,
                        requirements: std::slice::from_ref(&self.requirement),
                        scope: &self.scope,
                        owner_user_id: &self.owner_user_id,
                        run_id: self.run_id,
                        gate_ref: &self.gate_ref,
                    },
                    &self.requirement,
                )
                .await
                .unwrap()
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
}
