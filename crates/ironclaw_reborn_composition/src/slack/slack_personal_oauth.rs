//! Slack personal (user-token) OAuth: provider spec + blocked-gate provider.
//!
//! This is one of the two production [`OAuthGateProvider`] implementors. The
//! shared challenge/turn-gate-reuse/PKCE-store/cleanup/expiry logic lives in
//! [`crate::product_auth::oauth::oauth_gate::OAuthGateFlowDriver`]; only the flow preparation differs
//! from Google — Slack resolves client credentials from its setup slot at
//! request time and emits `user_scope=` (its `scope=` is reserved for bot
//! tokens) via the generic authorization-URL builder.

use std::{fmt, sync::Arc};

use axum::{
    Json,
    extract::{RawQuery, State},
    http::{HeaderMap, StatusCode, Uri},
    response::Response,
};
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthFlowRecord, AuthFlowRecordSource,
    AuthFlowStatus, AuthProductError, AuthProductScope, AuthProviderId, CredentialAccountLabel,
    OAuthAuthorizationEndpoint, OAuthAuthorizeUrlRequest, OAuthCallbackState,
    OAuthCallbackStateKind, OAuthProviderIdentity, OAuthScopeParam, PkceVerifierSecret,
    ProviderScope, SLACK_PERSONAL_AUTHORIZATION_ENDPOINT, SLACK_PERSONAL_PROVIDER_ID,
    build_authorization_url_with_scope_param, opaque_state_hash, pkce_s256_challenge,
    pkce_verifier_hash,
};
use ironclaw_host_api::ExtensionId;
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use secrecy::{ExposeSecret, SecretString};

use crate::extension_host::available_extensions::{
    SLACK_EXTENSION_ID, slack_personal_oauth_setup_scopes,
};
use crate::product_auth::api::auth::{
    OAuthProviderIdentityBindingRollback, OAuthProviderIdentityCheck,
    OAuthProviderIdentityCheckFuture, RebornOAuthCallbackFailureStage, RebornOAuthStartFlowRequest,
};
use crate::product_auth::oauth::oauth_gate::{OAuthGateProvider, PreparedOAuthGateFlow};
use crate::product_auth::oauth::oauth_provider_client::{
    ExchangeScopePolicy, HostOAuthProviderSpec, TokenResponseShape,
};
use crate::product_auth::serve::{
    CallbackScopeResolution, ExtensionOAuthStartRequest, OAuthCallbackDescriptor,
    OAuthCallbackTerminalHookFuture, PRODUCT_AUTH_FLOW_MAX_TTL_SECONDS, ProductAuthRouteFailure,
    ProductAuthRouteState, ProductOAuthStartResponse, ScopeFields, oauth_provider_callback_handler,
    opaque_state_hash as route_opaque_state_hash, pkce_verifier_hash as route_pkce_verifier_hash,
    run_with_backend_timeout, scope_from_authenticated_caller_parts_requiring_invocation,
    scope_hint, scoped_update_binding_for_requester,
};
use crate::slack::slack_host_beta::SlackPersonalConnectionScopeResolver;
use crate::slack::slack_personal_binding::{
    RebornUserIdentityBindingError, SlackConnectionEpoch, SlackConnectionOwner,
    SlackConnectionState, SlackPersonalBindingPrincipal, SlackPersonalUserBindingError,
    SlackPersonalUserBindingRequest, SlackUserBindingLifecycleError,
};
use crate::slack::slack_serve::{SlackApiAppId, SlackEnterpriseId, SlackTeamId, SlackUserId};
use crate::slack::slack_setup::SlackPersonalSetupServiceSlot;

/// Late-filled Slack-only lifecycle ports used by blocked-turn OAuth starts.
/// The OAuth provider registry is composed before the Slack host mounts exist,
/// so this travels through the same lazy slot as the setup credentials.
#[derive(Clone)]
pub(crate) struct SlackPersonalOAuthGateLifecycle {
    connection_scope_resolver: Arc<dyn SlackPersonalConnectionScopeResolver>,
    lifecycle_store: Arc<dyn crate::slack::slack_personal_binding::SlackUserBindingLifecycleStore>,
}

impl SlackPersonalOAuthGateLifecycle {
    pub(crate) fn new(
        connection_scope_resolver: Arc<dyn SlackPersonalConnectionScopeResolver>,
        lifecycle_store: Arc<
            dyn crate::slack::slack_personal_binding::SlackUserBindingLifecycleStore,
        >,
    ) -> Self {
        Self {
            connection_scope_resolver,
            lifecycle_store,
        }
    }
}

impl fmt::Debug for SlackPersonalOAuthGateLifecycle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SlackPersonalOAuthGateLifecycle")
            .finish_non_exhaustive()
    }
}

/// Host OAuth provider spec for the Slack personal (user-token) provider.
///
/// `SlackAuthedUser` response shape so the exchanger extracts the user token
/// from `authed_user.access_token`. Slack does not return granted scopes in a
/// standard `scope` field, so the exchange falls back to the requested scopes.
pub(crate) fn slack_personal_provider_spec() -> HostOAuthProviderSpec {
    HostOAuthProviderSpec {
        provider_id: SLACK_PERSONAL_PROVIDER_ID,
        capability_id: "ironclaw_auth.slack_personal_oauth",
        token_endpoint: ironclaw_auth::SLACK_PERSONAL_TOKEN_ENDPOINT,
        secret_handle_prefix: "slack_personal",
        resource: None,
        exchange_scope_policy: ExchangeScopePolicy::FallbackToRequested,
        token_response_shape: TokenResponseShape::SlackAuthedUser,
    }
}

/// Start the Slack-owned extension OAuth flow from the shared route boundary.
/// Provider-neutral flow persistence and callback processing remain in
/// product-auth; Slack owns only its URL shape and binding lifecycle.
pub(crate) async fn start_extension_oauth_flow(
    state: ProductAuthRouteState,
    caller: WebUiAuthenticatedCaller,
    request: ExtensionOAuthStartRequest,
    requester_extension: ExtensionId,
) -> Result<Json<ProductOAuthStartResponse>, ProductAuthRouteFailure> {
    let now = Utc::now();
    if request.expires_at <= now
        || request.expires_at > now + ChronoDuration::seconds(PRODUCT_AUTH_FLOW_MAX_TTL_SECONDS)
    {
        return Err(ProductAuthRouteFailure::invalid_request());
    }

    let (client_id, redirect_uri) = state.slack_personal_oauth_credentials().await?;
    if requester_extension.as_str() != SLACK_EXTENSION_ID {
        return Err(ProductAuthRouteFailure::invalid_request());
    }
    let internal_invariant = |field: &'static str| {
        tracing::error!(
            field,
            "slack personal OAuth start hit an invalid built-in constant"
        );
        ProductAuthRouteFailure::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            AuthErrorCode::BackendUnavailable,
        )
    };
    let provider = AuthProviderId::new(SLACK_PERSONAL_PROVIDER_ID)
        .map_err(|_| internal_invariant("provider_id"))?;
    let account_label = CredentialAccountLabel::new(request.account_label)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let requested_scopes = slack_personal_oauth_setup_scopes()
        .iter()
        .copied()
        .map(ProviderScope::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| internal_invariant("provider_scopes"))?;
    let fields = ScopeFields {
        session_id: None,
        thread_id: None,
        invocation_id: request.invocation_id,
    };
    let scope = scope_from_authenticated_caller_parts_requiring_invocation(&caller, &fields)?;
    let flow_id = AuthFlowId::new();
    let update_binding = scoped_update_binding_for_requester(
        &state,
        scope.clone(),
        provider.clone(),
        Some(&requester_extension),
    )
    .await?;
    let opaque_state = OAuthCallbackState::new(
        OAuthCallbackStateKind::SLACK_PERSONAL,
        flow_id,
        scope.clone(),
        account_label,
        requested_scopes.clone(),
    )
    .map_err(ProductAuthRouteFailure::from)?
    .encode()
    .map_err(ProductAuthRouteFailure::from)?;
    let opaque_state_hash = route_opaque_state_hash(opaque_state.as_str())?;
    let pkce_verifier_secret = SecretString::from(ironclaw_common::pkce::generate_code_verifier());
    let pkce_verifier_hash = route_pkce_verifier_hash(pkce_verifier_secret.expose_secret())?;
    let pkce_secret = PkceVerifierSecret::new(pkce_verifier_secret.clone())
        .map_err(ProductAuthRouteFailure::from)?;
    let pkce_challenge = pkce_s256_challenge(&pkce_secret);
    let authorization_endpoint =
        OAuthAuthorizationEndpoint::new(SLACK_PERSONAL_AUTHORIZATION_ENDPOINT)
            .map_err(ProductAuthRouteFailure::from)?;
    let authorization_url = build_authorization_url_with_scope_param(
        OAuthAuthorizeUrlRequest {
            authorization_endpoint: &authorization_endpoint,
            client_id: &client_id,
            redirect_uri: &redirect_uri,
            state: &opaque_state,
            code_challenge: &pkce_challenge,
            scopes: &requested_scopes,
            extra_params: &[],
        },
        OAuthScopeParam::UserScope,
    )
    .map_err(ProductAuthRouteFailure::from)?;

    let binding_config = state
        .slack_personal_oauth_binding_config()
        .ok_or_else(ProductAuthRouteFailure::backend_unavailable)?;
    let connection_scope = binding_config
        .connection_scope_resolver
        .resolve_personal_connection_scope()
        .await
        .map_err(|_| ProductAuthRouteFailure::backend_unavailable())?
        .ok_or_else(ProductAuthRouteFailure::backend_unavailable)?;
    let connection_owner = SlackConnectionOwner::new(
        scope.resource.tenant_id.clone(),
        scope.resource.user_id.clone(),
        connection_scope.installation_id,
    );
    let connection_epoch = SlackConnectionEpoch::new(flow_id);

    let flow = match run_with_backend_timeout(
        state
            .product_auth_services()
            .start_setup_oauth_flow(RebornOAuthStartFlowRequest {
                flow_id: Some(flow_id),
                scope: scope.clone(),
                provider: provider.clone(),
                authorization_url: authorization_url.clone(),
                opaque_state_hash: opaque_state_hash.clone(),
                pkce_verifier_hash,
                continuation: AuthContinuationRef::LifecycleActivation {
                    package_ref: ironclaw_auth::LifecyclePackageRef::new(
                        requester_extension.as_str(),
                    )
                    .map_err(|_| internal_invariant("lifecycle_package_ref"))?,
                },
                update_binding,
                expires_at: request.expires_at,
            }),
    )
    .await
    {
        Ok(flow) => flow,
        Err(error) => return Err(error),
    };
    if let Err(error) = state.store_pkce_verifier(flow.id, pkce_verifier_secret, flow.expires_at) {
        let _ = state
            .product_auth_services()
            .flow_manager()
            .cancel_flow(&scope, flow.id)
            .await;
        return Err(error);
    }
    if let Err(error) = binding_config
        .lifecycle_store
        .begin_connection(&connection_owner, connection_epoch, request.expires_at)
        .await
    {
        state.remove_pkce_verifier(flow.id);
        let _ = state
            .product_auth_services()
            .flow_manager()
            .cancel_flow(&scope, flow.id)
            .await;
        return Err(slack_lifecycle_start_failure(error));
    }

    Ok(Json(ProductOAuthStartResponse {
        flow_id: flow.id,
        status: flow.status,
        provider,
        authorization_url,
        expires_at: flow.expires_at,
        continuation: flow.continuation,
        callback_scope: scope_hint(&scope),
    }))
}

pub(crate) fn slack_lifecycle_start_failure(
    error: SlackUserBindingLifecycleError,
) -> ProductAuthRouteFailure {
    match error {
        SlackUserBindingLifecycleError::ConnectionInProgress
        | SlackUserBindingLifecycleError::DisconnectInProgress => {
            ProductAuthRouteFailure::new(StatusCode::CONFLICT, AuthErrorCode::ConnectionConflict)
        }
        SlackUserBindingLifecycleError::StaleEpoch | SlackUserBindingLifecycleError::Backend(_) => {
            ProductAuthRouteFailure::backend_unavailable()
        }
    }
}

pub(crate) static SLACK_PERSONAL_CALLBACK_DESCRIPTOR: OAuthCallbackDescriptor =
    OAuthCallbackDescriptor {
        state_kind: OAuthCallbackStateKind::SLACK_PERSONAL,
        provider_id: SLACK_PERSONAL_PROVIDER_ID,
        scope_resolution: CallbackScopeResolution::RequestedOnly,
        identity_hook: slack_personal_identity_hook,
        on_terminal_failure: Some(slack_personal_oauth_abandon_hook),
    };

pub(crate) async fn slack_personal_oauth_callback_handler(
    State(state): State<ProductAuthRouteState>,
    RawQuery(raw_query): RawQuery,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ProductAuthRouteFailure> {
    oauth_provider_callback_handler(
        state,
        &SLACK_PERSONAL_CALLBACK_DESCRIPTOR,
        raw_query,
        uri,
        headers,
    )
    .await
}

fn slack_personal_oauth_abandon_hook(
    state: ProductAuthRouteState,
    callback_scope: AuthProductScope,
    flow_id: AuthFlowId,
    failure_stage: RebornOAuthCallbackFailureStage,
) -> OAuthCallbackTerminalHookFuture {
    Box::pin(async move {
        let Some(config) = state.slack_personal_oauth_binding_config() else {
            tracing::warn!(
                %flow_id,
                "Slack terminal cleanup authority is unavailable; keeping flow retryable"
            );
            return Err(ProductAuthRouteFailure::backend_unavailable());
        };
        let connection_epoch = SlackConnectionEpoch::new(flow_id);
        let connection_owner = match config
            .lifecycle_store
            .connection_owner_for_epoch(
                &callback_scope.resource.tenant_id,
                &callback_scope.resource.user_id,
                connection_epoch,
            )
            .await
        {
            Ok(Some(connection_owner)) => connection_owner,
            Ok(None) => return Ok(()),
            Err(error) => {
                tracing::warn!(
                    %error,
                    flow_id = %flow_id,
                    "failed to inspect terminal Slack OAuth connection epoch"
                );
                return Err(ProductAuthRouteFailure::backend_unavailable());
            }
        };
        let provider_user_id_prefix = format!("{}:", connection_owner.installation_id().as_str());
        if matches!(
            failure_stage,
            RebornOAuthCallbackFailureStage::ContinuationSideEffect
                | RebornOAuthCallbackFailureStage::ContinuationCompensation
        ) {
            if let Err(error) = config
                .lifecycle_store
                .begin_failed_connection_cleanup(&connection_owner, connection_epoch)
                .await
            {
                tracing::warn!(
                    %error,
                    flow_id = %flow_id,
                    "failed to fence terminal Slack OAuth epoch before identity cleanup"
                );
                return Err(ProductAuthRouteFailure::backend_unavailable());
            }
            if let Err(error) = config
                .binding_rollback_store
                .delete_user_identity_bindings_for_user_at_epoch(
                    crate::slack::slack_actor_identity::SLACK_IDENTITY_PROVIDER,
                    &callback_scope.resource.user_id,
                    Some(provider_user_id_prefix.as_str()),
                    Some(connection_epoch),
                )
                .await
            {
                tracing::warn!(
                    %error,
                    flow_id = %flow_id,
                    "retaining Slack OAuth lifecycle owner because failed activation identity cleanup did not complete"
                );
                return Err(ProductAuthRouteFailure::backend_unavailable());
            }
            if let Err(error) = config
                .lifecycle_store
                .complete_failed_connection_cleanup(&connection_owner, connection_epoch)
                .await
            {
                tracing::warn!(
                    %error,
                    flow_id = %flow_id,
                    "failed to settle terminal Slack OAuth epoch after identity cleanup"
                );
                return Err(ProductAuthRouteFailure::backend_unavailable());
            }
            return Ok(());
        }
        match config
            .binding_rollback_store
            .user_identity_bindings_for_user_at_epoch(
                crate::slack::slack_actor_identity::SLACK_IDENTITY_PROVIDER,
                &callback_scope.resource.user_id,
                Some(provider_user_id_prefix.as_str()),
                Some(connection_epoch),
            )
            .await
        {
            Ok(bindings) if !bindings.is_empty() => {
                tracing::debug!(
                    flow_id = %flow_id,
                    "retaining Slack OAuth lifecycle owner because identity cleanup is still pending"
                );
                return Err(ProductAuthRouteFailure::backend_unavailable());
            }
            Err(error) => {
                tracing::debug!(
                    %error,
                    flow_id = %flow_id,
                    "retaining Slack OAuth lifecycle owner because identity cleanup could not be verified"
                );
                return Err(ProductAuthRouteFailure::backend_unavailable());
            }
            Ok(_) => {}
        }
        abandon_slack_connection_epoch(config.lifecycle_store.as_ref(), &callback_scope, flow_id)
            .await
    })
}

async fn abandon_slack_connection_epoch(
    lifecycle_store: &dyn crate::slack::slack_personal_binding::SlackUserBindingLifecycleStore,
    scope: &AuthProductScope,
    flow_id: AuthFlowId,
) -> Result<(), ProductAuthRouteFailure> {
    let connection_epoch = SlackConnectionEpoch::new(flow_id);
    let connection_owner = match lifecycle_store
        .connection_owner_for_epoch(
            &scope.resource.tenant_id,
            &scope.resource.user_id,
            connection_epoch,
        )
        .await
    {
        Ok(Some(connection_owner)) => connection_owner,
        Ok(None) => return Ok(()),
        Err(error) => {
            tracing::warn!(%error, %flow_id, "failed to inspect terminal Slack OAuth epoch");
            return Err(ProductAuthRouteFailure::backend_unavailable());
        }
    };
    if let Err(error) = lifecycle_store
        .abandon_connection(&connection_owner, connection_epoch)
        .await
    {
        tracing::warn!(%error, "failed to abandon terminal Slack OAuth connection epoch");
        return Err(ProductAuthRouteFailure::backend_unavailable());
    }
    Ok(())
}

fn slack_personal_identity_hook(
    state: &ProductAuthRouteState,
    callback_scope: &AuthProductScope,
    flow_id: AuthFlowId,
) -> Option<OAuthProviderIdentityCheck> {
    let state = state.clone();
    let callback_scope = callback_scope.clone();
    Some(Box::new(
        move |provider_identity: Option<OAuthProviderIdentity>| {
            Box::pin(async move {
                bind_slack_personal_oauth_identity_for_callback(
                    &state,
                    &callback_scope,
                    flow_id,
                    provider_identity.as_ref(),
                )
                .await
                .map(Some)
            }) as OAuthProviderIdentityCheckFuture
        },
    ))
}

async fn bind_slack_personal_oauth_identity_for_callback(
    state: &ProductAuthRouteState,
    callback_scope: &AuthProductScope,
    flow_id: AuthFlowId,
    provider_identity: Option<&OAuthProviderIdentity>,
) -> Result<OAuthProviderIdentityBindingRollback, AuthProductError> {
    let Some(config) = state.slack_personal_oauth_binding_config() else {
        tracing::debug!(
            "Slack personal OAuth callback reached without a binding config; failing closed"
        );
        return Err(AuthProductError::BackendUnavailable);
    };
    let identity = provider_identity.ok_or(AuthProductError::MalformedCallback)?;
    let connection_epoch = SlackConnectionEpoch::new(flow_id);
    let connection_owner = config
        .lifecycle_store
        .connection_owner_for_epoch(
            &callback_scope.resource.tenant_id,
            &callback_scope.resource.user_id,
            connection_epoch,
        )
        .await
        .map_err(|error| {
            tracing::debug!(%error, "Slack personal OAuth binding owner lookup failed");
            AuthProductError::BackendUnavailable
        })?
        .ok_or(AuthProductError::BackendUnavailable)?;
    let team_id = identity
        .team_id
        .as_ref()
        .ok_or(AuthProductError::MalformedCallback)?;
    let api_app_id = identity
        .app_id
        .as_ref()
        .ok_or(AuthProductError::MalformedCallback)?;
    let enterprise_id = identity
        .enterprise_id
        .as_ref()
        .map(|value| SlackEnterpriseId::new(value.clone()));
    let outcome = match config
        .binding_service
        .bind_personal_user_for_epoch(
            SlackPersonalBindingPrincipal {
                tenant_id: callback_scope.resource.tenant_id.clone(),
                user_id: callback_scope.resource.user_id.clone(),
            },
            SlackPersonalUserBindingRequest {
                installation_id: connection_owner.installation_id().clone(),
                slack_user_id: SlackUserId::new(identity.subject.as_str()),
                team_id: SlackTeamId::new(team_id.clone()),
                enterprise_id,
                api_app_id: SlackApiAppId::new(api_app_id.clone()),
            },
            connection_epoch,
        )
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            let _ = config
                .lifecycle_store
                .abandon_connection(&connection_owner, connection_epoch)
                .await;
            return Err(slack_personal_user_binding_auth_error(error));
        }
    };
    Ok(outcome.rollback.into_future())
}

fn slack_personal_user_binding_auth_error(
    error: SlackPersonalUserBindingError,
) -> AuthProductError {
    match error {
        SlackPersonalUserBindingError::UnknownInstallation { .. }
        | SlackPersonalUserBindingError::InstallationNotTenantScoped { .. }
        | SlackPersonalUserBindingError::SlackInstallationContextMismatch { .. }
        | SlackPersonalUserBindingError::InvalidSlackId { .. } => {
            AuthProductError::MalformedCallback
        }
        SlackPersonalUserBindingError::BindingStore(
            RebornUserIdentityBindingError::ProviderIdentityAlreadyBound,
        ) => AuthProductError::ProviderIdentityAlreadyConnected,
        SlackPersonalUserBindingError::BindingStore(_) => AuthProductError::BackendUnavailable,
    }
}

/// Slack personal (user-token) blocked-turn OAuth gate provider.
///
/// Holds the Slack setup slot; the shared [`crate::product_auth::oauth::oauth_gate::OAuthGateFlowDriver`]
/// owns everything else.
#[derive(Clone)]
pub(crate) struct SlackPersonalOAuthGateProvider {
    slot: SlackPersonalSetupServiceSlot,
}

impl SlackPersonalOAuthGateProvider {
    pub(crate) fn new(slot: SlackPersonalSetupServiceSlot) -> Self {
        Self { slot }
    }

    async fn connection_owner(
        &self,
        scope: &AuthProductScope,
    ) -> Result<(SlackPersonalOAuthGateLifecycle, SlackConnectionOwner), AuthProductError> {
        let lifecycle = self
            .slot
            .gate_lifecycle()
            .ok_or(AuthProductError::BackendUnavailable)?;
        let connection_scope = lifecycle
            .connection_scope_resolver
            .resolve_personal_connection_scope()
            .await
            .map_err(|error| {
                tracing::debug!(%error, "Slack personal OAuth connection scope unavailable");
                AuthProductError::BackendUnavailable
            })?
            .ok_or(AuthProductError::BackendUnavailable)?;
        let owner = SlackConnectionOwner::new(
            scope.resource.tenant_id.clone(),
            scope.resource.user_id.clone(),
            connection_scope.installation_id,
        );
        Ok((lifecycle, owner))
    }
}

#[async_trait::async_trait]
impl OAuthGateProvider for SlackPersonalOAuthGateProvider {
    fn provider_id(&self) -> &'static str {
        SLACK_PERSONAL_PROVIDER_ID
    }

    fn pkce_secret_handle_label(&self) -> &'static str {
        "slack-personal-oauth-gate-flow-pkce"
    }

    async fn select_reusable_flow(
        &self,
        scope: &AuthProductScope,
        exact: Option<AuthFlowRecord>,
        flow_source: &dyn AuthFlowRecordSource,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        let (lifecycle, connection_owner) = self.connection_owner(scope).await?;
        let connection_state = lifecycle
            .lifecycle_store
            .connection_state(&connection_owner)
            .await
            .map_err(slack_lifecycle_gate_failure)?;
        if connection_state.is_none()
            && let Some(exact) = exact.as_ref()
            && exact.provider.as_str() == SLACK_PERSONAL_PROVIDER_ID
            && exact.status == AuthFlowStatus::AwaitingUser
            && exact.expires_at > Utc::now()
        {
            lifecycle
                .lifecycle_store
                .begin_connection(
                    &connection_owner,
                    SlackConnectionEpoch::new(exact.id),
                    exact.expires_at,
                )
                .await
                .map_err(slack_lifecycle_gate_failure)?;
        }
        let Some((epoch, SlackConnectionState::Connecting)) = lifecycle
            .lifecycle_store
            .connection_state(&connection_owner)
            .await
            .map_err(slack_lifecycle_gate_failure)?
        else {
            return Ok(None);
        };
        let now = Utc::now();
        let flow = match exact {
            Some(flow) if flow.id == epoch.flow_id() => Some(flow),
            Some(_) | None => {
                flow_source
                    .flow_for_owner_by_id(scope, epoch.flow_id())
                    .await?
            }
        };
        if lifecycle
            .lifecycle_store
            .connection_state(&connection_owner)
            .await
            .map_err(slack_lifecycle_gate_failure)?
            != Some((epoch, SlackConnectionState::Connecting))
        {
            return Ok(None);
        }
        match flow {
            Some(flow)
                if flow.provider.as_str() == SLACK_PERSONAL_PROVIDER_ID
                    && flow.status == AuthFlowStatus::AwaitingUser
                    && flow.expires_at > now =>
            {
                Ok(Some(flow))
            }
            Some(flow)
                if flow.provider.as_str() == SLACK_PERSONAL_PROVIDER_ID
                    && (matches!(
                        flow.status,
                        AuthFlowStatus::Canceled | AuthFlowStatus::Expired | AuthFlowStatus::Failed
                    ) || flow.expires_at <= now) =>
            {
                lifecycle
                    .lifecycle_store
                    .abandon_connection(&connection_owner, epoch)
                    .await
                    .map_err(slack_lifecycle_gate_failure)?;
                Ok(None)
            }
            Some(_) | None => Ok(None),
        }
    }

    async fn prepare_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        scopes: Vec<ProviderScope>,
        _expires_at: ironclaw_auth::Timestamp,
    ) -> Result<PreparedOAuthGateFlow, AuthProductError> {
        let service = self
            .slot
            .get()
            .ok_or(AuthProductError::BackendUnavailable)?;
        let (client_id, _client_secret) = service.oauth_credentials().await.map_err(|e| {
            tracing::debug!(error = %e, "Slack personal OAuth credentials not configured");
            AuthProductError::BackendUnavailable
        })?;
        let account_label = CredentialAccountLabel::new("slack_personal")?;
        let state = OAuthCallbackState::new(
            OAuthCallbackStateKind::SLACK_PERSONAL,
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
        let authorization_endpoint =
            OAuthAuthorizationEndpoint::new(SLACK_PERSONAL_AUTHORIZATION_ENDPOINT)?;
        let authorization_url = build_authorization_url_with_scope_param(
            OAuthAuthorizeUrlRequest {
                authorization_endpoint: &authorization_endpoint,
                client_id: &client_id,
                redirect_uri: self.slot.redirect_uri(),
                state: &state,
                code_challenge: &pkce_challenge,
                scopes: &scopes,
                extra_params: &[],
            },
            OAuthScopeParam::UserScope,
        )?;
        Ok(PreparedOAuthGateFlow {
            authorization_url,
            opaque_state_hash,
            pkce_verifier_hash,
            pkce_verifier,
        })
    }

    async fn publish_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        expires_at: ironclaw_auth::Timestamp,
    ) -> Result<(), AuthProductError> {
        let (lifecycle, connection_owner) = self.connection_owner(scope).await?;
        lifecycle
            .lifecycle_store
            .begin_connection(
                &connection_owner,
                SlackConnectionEpoch::new(flow_id),
                expires_at,
            )
            .await
            .map_err(slack_lifecycle_gate_failure)
    }

    async fn abandon_flow(&self, scope: &AuthProductScope, flow_id: AuthFlowId) {
        let Some(lifecycle) = self.slot.gate_lifecycle() else {
            return;
        };
        if let Err(error) =
            abandon_slack_connection_epoch(lifecycle.lifecycle_store.as_ref(), scope, flow_id).await
        {
            tracing::warn!(
                %flow_id,
                ?error,
                "Slack OAuth flow abandonment remains pending"
            );
        }
    }
}

fn slack_lifecycle_gate_failure(error: SlackUserBindingLifecycleError) -> AuthProductError {
    match error {
        SlackUserBindingLifecycleError::ConnectionInProgress
        | SlackUserBindingLifecycleError::DisconnectInProgress => AuthProductError::BackendConflict,
        SlackUserBindingLifecycleError::StaleEpoch | SlackUserBindingLifecycleError::Backend(_) => {
            AuthProductError::BackendUnavailable
        }
    }
}

impl fmt::Debug for SlackPersonalOAuthGateProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SlackPersonalOAuthGateProvider")
            .field("slot", &self.slot)
            .finish()
    }
}
