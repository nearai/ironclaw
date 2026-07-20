//! Slack personal (user-token) OAuth: provider spec + blocked-gate provider.
//!
//! This is one of the two production [`OAuthGateProvider`] implementors. The
//! shared challenge/turn-gate-reuse/PKCE-store/cleanup/expiry logic lives in
//! [`crate::product_auth::oauth::oauth_gate::OAuthGateFlowDriver`]; only the flow preparation differs
//! from Google — Slack resolves client credentials from its setup slot at
//! request time and emits `user_scope=` (its `scope=` is reserved for bot
//! tokens) via the generic authorization-URL builder.

use std::fmt;

use axum::{
    Json,
    extract::{RawQuery, State},
    http::{HeaderMap, StatusCode, Uri},
    response::Response,
};
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthProductError, AuthProductScope,
    AuthProviderId, CredentialAccountLabel, OAuthAuthorizationEndpoint, OAuthAuthorizationUrl,
    OAuthAuthorizeUrlRequest, OAuthCallbackState, OAuthCallbackStateKind, OAuthExtraParam,
    OAuthProviderIdentity, OAuthRedirectUri, OAuthScopeParam, OAuthState, PkceCodeChallenge,
    PkceVerifierSecret, ProviderScope, SLACK_PERSONAL_AUTHORIZATION_ENDPOINT,
    SLACK_PERSONAL_PROVIDER_ID, build_authorization_url_with_scope_param, opaque_state_hash,
    pkce_s256_challenge, pkce_verifier_hash,
};
use ironclaw_host_api::ExtensionId;
use ironclaw_product_adapters::AdapterInstallationId;
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
use crate::slack::slack_personal_binding::{
    RebornUserIdentityBindingError, SlackConnectionEpoch, SlackConnectionOwner,
    SlackPersonalBindingPrincipal, SlackPersonalUserBindingError, SlackPersonalUserBindingRequest,
    SlackUserBindingLifecycleError,
};
use crate::slack::slack_serve::{SlackApiAppId, SlackEnterpriseId, SlackTeamId, SlackUserId};
use crate::slack::slack_setup::{SlackOAuthAuthorizationContext, SlackPersonalSetupServiceSlot};

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

fn slack_personal_authorization_url(
    authorization: &SlackOAuthAuthorizationContext,
    redirect_uri: &OAuthRedirectUri,
    state: &OAuthState,
    code_challenge: &PkceCodeChallenge,
    scopes: &[ProviderScope],
) -> Result<OAuthAuthorizationUrl, AuthProductError> {
    let team_param = OAuthExtraParam::new("team", authorization.team_id.as_str())?;
    let extra_params = [team_param];
    let authorization_endpoint =
        OAuthAuthorizationEndpoint::new(SLACK_PERSONAL_AUTHORIZATION_ENDPOINT)?;
    build_authorization_url_with_scope_param(
        OAuthAuthorizeUrlRequest {
            authorization_endpoint: &authorization_endpoint,
            client_id: &authorization.client_id,
            redirect_uri,
            state,
            code_challenge,
            scopes,
            extra_params: &extra_params,
        },
        OAuthScopeParam::UserScope,
    )
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

    let (authorization, redirect_uri) = state.slack_personal_oauth_authorization_context().await?;
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
    let authorization_url = slack_personal_authorization_url(
        &authorization,
        &redirect_uri,
        &opaque_state,
        &pkce_challenge,
        &requested_scopes,
    )
    .map_err(ProductAuthRouteFailure::from)?;

    // The start path keeps NO Slack connection state: attempt liveness is the
    // auth-flow record alone (created below, which supersedes any prior live
    // setup-class flow at the `create_flow` seam), and the connection record
    // is written by the callback's identity bind where the durable generation
    // actually begins. Resolving the connection scope here is a fail-closed
    // configuration check only — the callback's identity hook will need this
    // same resolution to bind, so a missing Slack channel mount fails the
    // start instead of minting a flow whose callback cannot complete.
    let binding_config = state
        .slack_personal_oauth_binding_config()
        .ok_or_else(ProductAuthRouteFailure::backend_unavailable)?;
    binding_config
        .connection_scope_resolver
        .resolve_personal_connection_scope()
        .await
        .map_err(|error| {
            tracing::debug!(%error, "Slack personal OAuth connection scope unavailable");
            ProductAuthRouteFailure::backend_unavailable()
        })?
        .ok_or_else(ProductAuthRouteFailure::backend_unavailable)?;

    let flow = run_with_backend_timeout(
        state
            .product_auth_services()
            .start_setup_oauth_flow(RebornOAuthStartFlowRequest {
                flow_id: Some(flow_id),
                scope: scope.clone(),
                provider: provider.clone(),
                authorization_url: authorization_url.clone(),
                opaque_state_hash: opaque_state_hash.clone(),
                pkce_verifier_hash,
                pkce_verifier: pkce_verifier_secret,
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
    .await?;

    Ok(Json(ProductOAuthStartResponse {
        flow_id: flow.id,
        status: flow.state.into(),
        provider,
        authorization_url,
        expires_at: flow.expires_at,
        continuation: flow.continuation,
        callback_scope: scope_hint(&scope),
    }))
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

/// Terminal-failure cleanup for the Slack personal callback.
///
/// The identity bind stamps rows with the flow id and records the active
/// generation. A terminal callback failure runs the journaled failed-connection
/// sweep so that generation's rows are removed durably even across a crash.
/// A retryable continuation failure is deliberately ignored here: the OAuth
/// result remains authorized and its exact durable resolution must be retried,
/// not torn down.
///
/// Cleanup authority is the stamped rows themselves: each row's provider user
/// id carries the installation the bind actually wrote under. Re-resolving the
/// connection scope here could drift (the operator repointing Slack setup
/// between the bind and this hook) and would sweep an owner this generation
/// never touched, orphaning the real rows. When rows exist they are reclaimed
/// through the journaled sweep whatever the reported stage — a post-bind
/// completion failure classifies as `Terminal` — and the resolver is consulted
/// only when no rows survive, to settle a possibly still-active generation
/// record.
fn slack_personal_oauth_abandon_hook(
    state: ProductAuthRouteState,
    callback_scope: AuthProductScope,
    flow_id: AuthFlowId,
    failure_stage: RebornOAuthCallbackFailureStage,
) -> OAuthCallbackTerminalHookFuture {
    Box::pin(async move {
        if matches!(
            failure_stage,
            RebornOAuthCallbackFailureStage::ContinuationRetryable
        ) {
            return Ok(());
        }
        let Some(config) = state.slack_personal_oauth_binding_config() else {
            tracing::warn!(
                %flow_id,
                "Slack terminal cleanup authority is unavailable; keeping flow retryable"
            );
            return Err(ProductAuthRouteFailure::backend_unavailable());
        };
        let connection_epoch = SlackConnectionEpoch::new(flow_id);
        let stamped_rows = match config
            .binding_rollback_store
            .user_identity_bindings_for_user_at_epoch(
                crate::slack::slack_actor_identity::SLACK_IDENTITY_PROVIDER,
                &callback_scope.resource.user_id,
                None,
                Some(connection_epoch),
            )
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::debug!(
                    %error,
                    flow_id = %flow_id,
                    "retaining Slack OAuth flow because identity cleanup could not be verified"
                );
                return Err(ProductAuthRouteFailure::backend_unavailable());
            }
        };
        let mut stamped_installations: Vec<AdapterInstallationId> = Vec::new();
        for row in &stamped_rows {
            let provider_user_id = row.binding().provider_user_id.as_str();
            let Some((installation_id, _)) =
                crate::slack::slack_actor_identity::parse_slack_user_identity_provider_user_id(
                    provider_user_id,
                )
            else {
                // A stamped row whose installation cannot be derived cannot be
                // reclaimed by any owner-scoped sweep; keep the flow retryable
                // rather than settling it as cleaned.
                tracing::warn!(
                    flow_id = %flow_id,
                    provider_user_id,
                    "retaining Slack OAuth flow because a stamped identity row names no installation"
                );
                return Err(ProductAuthRouteFailure::backend_unavailable());
            };
            if !stamped_installations.contains(&installation_id) {
                stamped_installations.push(installation_id);
            }
        }
        if stamped_installations.is_empty() {
            // Nothing stamped by this generation survives (the in-process
            // rollback unwound it, or the bind never landed). The owner's
            // generation record may still hold this failed generation if the
            // rollback's settle was lost; settle it via the current
            // resolution — with no rows there is no bind-time authority to
            // prefer, and `Ok(None)` (setup removed) leaves nothing to settle.
            let connection_scope = match config
                .connection_scope_resolver
                .resolve_personal_connection_scope()
                .await
            {
                Ok(Some(connection_scope)) => connection_scope,
                Ok(None) => {
                    tracing::debug!(
                        flow_id = %flow_id,
                        "Slack terminal cleanup found no configured connection scope; nothing to clean"
                    );
                    return Ok(());
                }
                Err(error) => {
                    tracing::debug!(
                        %error,
                        flow_id = %flow_id,
                        "failed to resolve Slack connection scope for terminal cleanup"
                    );
                    return Err(ProductAuthRouteFailure::backend_unavailable());
                }
            };
            let connection_owner = SlackConnectionOwner::new(
                callback_scope.resource.tenant_id.clone(),
                callback_scope.resource.user_id.clone(),
                connection_scope.installation_id,
            );
            match config
                .lifecycle_store
                .begin_failed_connection_cleanup(&connection_owner, connection_epoch)
                .await
            {
                Ok(()) => {}
                // The generation record already moved on — the in-process
                // rollback restored a previous generation or settled it — and
                // with no stamped rows there is nothing left to reclaim.
                Err(SlackUserBindingLifecycleError::StaleEpoch) => return Ok(()),
                Err(error) => {
                    tracing::warn!(
                        %error,
                        flow_id = %flow_id,
                        "failed to fence terminal Slack OAuth epoch before identity cleanup"
                    );
                    return Err(ProductAuthRouteFailure::backend_unavailable());
                }
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
        // Rows stamped with this generation exist, so the bind committed
        // durable state regardless of the reported failure stage — a
        // post-bind completion failure surfaces as `Terminal` through the
        // blanket error conversion. Reclaim them through the journaled sweep,
        // targeting exactly the installations the rows name.
        for installation_id in stamped_installations {
            let connection_owner = SlackConnectionOwner::new(
                callback_scope.resource.tenant_id.clone(),
                callback_scope.resource.user_id.clone(),
                installation_id,
            );
            let provider_user_id_prefix =
                format!("{}:", connection_owner.installation_id().as_str());
            if let Err(error) = config
                .lifecycle_store
                .begin_failed_connection_cleanup(&connection_owner, connection_epoch)
                .await
            {
                // `StaleEpoch` with this generation's rows still stamped means
                // the owner's record moved past the generation while its rows
                // survive; fail closed (retryable) rather than sweeping
                // without the journal fence.
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
        }
        Ok(())
    })
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
    // The installation is resolved at bind time — the same authority the
    // start handler validated — and the proof-vs-selector check inside the
    // binding service rejects a callback whose Slack team/app no longer
    // matches it, so a setup drift between start and callback fails closed
    // instead of binding against stale configuration.
    let connection_scope = config
        .connection_scope_resolver
        .resolve_personal_connection_scope()
        .await
        .map_err(|error| {
            tracing::debug!(%error, "Slack personal OAuth connection scope unavailable");
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
    let outcome = config
        .binding_service
        .bind_personal_user_for_epoch(
            SlackPersonalBindingPrincipal {
                tenant_id: callback_scope.resource.tenant_id.clone(),
                user_id: callback_scope.resource.user_id.clone(),
            },
            SlackPersonalUserBindingRequest {
                installation_id: connection_scope.installation_id,
                slack_user_id: SlackUserId::new(identity.subject.as_str()),
                team_id: SlackTeamId::new(team_id.clone()),
                enterprise_id,
                api_app_id: SlackApiAppId::new(api_app_id.clone()),
            },
            connection_epoch,
        )
        .await
        .map_err(slack_personal_user_binding_auth_error)?;
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
/// owns everything else. Slack runs on the driver's default
/// `select_reusable_flow`/`publish_flow`/`abandon_flow` — like Google, it
/// keeps no provider-owned lifecycle state per flow: attempt liveness is the
/// auth-flow record, and the connection generation begins at the callback's
/// identity bind.
#[derive(Clone)]
pub(crate) struct SlackPersonalOAuthGateProvider {
    slot: SlackPersonalSetupServiceSlot,
}

impl SlackPersonalOAuthGateProvider {
    pub(crate) fn new(slot: SlackPersonalSetupServiceSlot) -> Self {
        Self { slot }
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
        let authorization = service.oauth_authorization_context().await.map_err(|e| {
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
        let authorization_url = slack_personal_authorization_url(
            &authorization,
            self.slot.redirect_uri(),
            &state,
            &pkce_challenge,
            &scopes,
        )?;
        Ok(PreparedOAuthGateFlow {
            authorization_url,
            opaque_state_hash,
            pkce_verifier_hash,
            pkce_verifier,
        })
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
