//! OAuth start and callback handlers.
// arch-exempt: large_file, product-auth OAuth start/callback serve surface; Slack-OAuth audit MJ4 will unify Slack personal-OAuth start here and shrink this file, plan #5604

use super::*;
#[cfg(feature = "slack-v2-host-beta")]
use crate::extension_host::available_extensions::{
    SLACK_EXTENSION_ID, slack_personal_oauth_setup_scopes,
};
use crate::product_auth::api::auth::OAuthProviderIdentityCheck;
#[cfg(feature = "slack-v2-host-beta")]
use crate::product_auth::api::auth::OAuthProviderIdentityCheckFuture;
use crate::product_auth::oauth::oauth_dcr::DcrOAuthCallbackState;
#[cfg(feature = "slack-v2-host-beta")]
use crate::slack::slack_personal_binding::{
    SlackPersonalBindingPrincipal, SlackPersonalUserBindingError, SlackPersonalUserBindingRequest,
};
#[cfg(feature = "slack-v2-host-beta")]
use crate::slack::slack_serve::{SlackApiAppId, SlackEnterpriseId, SlackTeamId, SlackUserId};
#[cfg(feature = "slack-v2-host-beta")]
use ironclaw_auth::OAuthProviderIdentity;

pub(super) async fn oauth_start_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<OAuthStartRequest>,
) -> Result<Json<OAuthStartResponse>, ProductAuthRouteFailure> {
    let now = Utc::now();
    if request.expires_at <= now
        || request.expires_at > now + ChronoDuration::seconds(PRODUCT_AUTH_FLOW_MAX_TTL_SECONDS)
    {
        return Err(ProductAuthRouteFailure::invalid_request());
    }

    let scope = scope_from_authenticated_caller(&caller, &request)?;
    let provider = AuthProviderId::new(request.provider).map_err(|_| {
        ProductAuthRouteFailure::new(StatusCode::BAD_REQUEST, AuthErrorCode::InvalidRequest)
    })?;
    let authorization_endpoint = authorization_endpoint_url(&request.authorization_url)?;
    let opaque_state = request
        .opaque_state
        .into_validated()
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let pkce_verifier_value = request
        .pkce_verifier
        .into_validated()
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let opaque_state_hash = opaque_state_hash(opaque_state.as_str())?;
    let pkce_verifier_hash = pkce_verifier_hash(pkce_verifier_value.expose_secret())?;
    let pkce_verifier = pkce_verifier_value.clone_secret();

    let flow = run_with_backend_timeout(
        state
            .product_auth
            .start_setup_oauth_flow(RebornOAuthStartFlowRequest {
                flow_id: None,
                scope: scope.clone(),
                provider: provider.clone(),
                authorization_url: OAuthAuthorizationUrl::new(authorization_endpoint.to_string())
                    .map_err(ProductAuthRouteFailure::from)?,
                opaque_state_hash,
                pkce_verifier_hash,
                update_binding: None,
                expires_at: request.expires_at,
            }),
    )
    .await?;
    state.store_pkce_verifier(flow.id, pkce_verifier, flow.expires_at)?;
    let authorization_url = compose_authorization_url(authorization_endpoint, flow.id, &scope)?;

    Ok(Json(OAuthStartResponse {
        flow_id: flow.id,
        status: flow.status,
        provider,
        authorization_url,
        expires_at: flow.expires_at,
        continuation: flow.continuation,
        callback_scope: scope_hint(&scope),
    }))
}

/// Origin-independent OAuth flow-status poll.
///
/// The callback page signals the opener via same-origin `localStorage` +
/// `BroadcastChannel`; a cross-origin callback (local ngrok callback vs
/// `127.0.0.1` opener, or split app/callback domains in prod) never reaches the
/// opener, so the reconnect watcher can hang. This read lets the watcher poll
/// durable flow status by id instead.
///
/// Caller-scoped: the trusted tenant/user/agent/project come from the
/// authenticated caller; the browser only echoes back the `invocation_id` the
/// start response minted so `get_flow`'s full-scope equality can locate the
/// caller's own flow. A flow that is unknown OR owned by another scope both
/// surface as `404 not_found` — never a 403 that would leak existence across
/// users. The response carries the sanitized status enum only: no tokens, PKCE
/// verifiers, authorization codes, or opaque state.
pub(super) async fn oauth_flow_status_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(flow_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<OAuthFlowStatusQuery>,
) -> Result<Json<OAuthFlowStatusResponse>, ProductAuthRouteFailure> {
    let flow_id = AuthFlowId::from_uuid(
        Uuid::parse_str(&flow_id).map_err(|_| ProductAuthRouteFailure::malformed_callback())?,
    );
    let fields = ScopeFields {
        session_id: None,
        thread_id: None,
        invocation_id: query.invocation_id,
    };
    let scope = scope_from_authenticated_caller_parts_requiring_invocation(&caller, &fields)?;
    let status = run_with_backend_timeout(state.product_auth.flow_status(&scope, flow_id)).await?;
    Ok(Json(OAuthFlowStatusResponse { status }))
}

pub(super) async fn google_oauth_start_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<GoogleOAuthStartRequest>,
) -> Result<Json<ProductOAuthStartResponse>, ProductAuthRouteFailure> {
    start_google_oauth_flow(state, caller, request, None, false).await
}

pub(super) async fn extension_oauth_start_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(package_id): Path<String>,
    Json(request): Json<ExtensionOAuthStartRequest>,
) -> Result<Json<ProductOAuthStartResponse>, ProductAuthRouteFailure> {
    let requester_extension =
        ExtensionId::new(package_id).map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    if request.provider == SLACK_PERSONAL_PROVIDER_ID {
        #[cfg(feature = "slack-v2-host-beta")]
        return start_slack_personal_oauth_flow(state, caller, request, requester_extension).await;
        #[cfg(not(feature = "slack-v2-host-beta"))]
        return Err(ProductAuthRouteFailure::backend_unavailable());
    }
    if request.provider != GOOGLE_PROVIDER_ID {
        return start_dcr_extension_oauth_flow(state, caller, request, requester_extension).await;
    }
    start_google_oauth_flow(
        state,
        caller,
        GoogleOAuthStartRequest {
            account_label: request.account_label,
            scopes: request.scopes,
            expires_at: request.expires_at,
            session_id: None,
            thread_id: None,
            invocation_id: request.invocation_id,
        },
        Some(requester_extension),
        true,
    )
    .await
}

async fn start_dcr_extension_oauth_flow(
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

    let provider = AuthProviderId::new(request.provider)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let account_label = CredentialAccountLabel::new(request.account_label)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let requested_scopes = request
        .scopes
        .into_iter()
        .map(ProviderScope::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let fields = ScopeFields {
        session_id: None,
        thread_id: None,
        invocation_id: request.invocation_id,
    };
    let scope = scope_from_authenticated_caller_parts_requiring_invocation(&caller, &fields)?;
    let update_binding = scoped_update_binding_for_requester(
        &state,
        scope.clone(),
        provider.clone(),
        Some(&requester_extension),
    )
    .await?;
    let flow = run_with_backend_timeout(state.product_auth.start_dcr_setup_oauth_flow(
        RebornDcrOAuthStartFlowRequest {
            scope: scope.clone(),
            provider: provider.clone(),
            account_label,
            provider_scopes: requested_scopes,
            update_binding,
            expires_at: request.expires_at,
        },
    ))
    .await?
    .ok_or_else(ProductAuthRouteFailure::malformed_config)?;
    let Some(AuthChallenge::OAuthUrl {
        authorization_url, ..
    }) = &flow.challenge
    else {
        return Err(ProductAuthRouteFailure::backend_unavailable());
    };

    Ok(Json(ProductOAuthStartResponse {
        flow_id: flow.id,
        status: flow.status,
        provider,
        authorization_url: authorization_url.clone(),
        expires_at: flow.expires_at,
        continuation: flow.continuation,
        callback_scope: scope_hint(&scope),
    }))
}

/// Non-DCR extension OAuth start for the Slack personal (user-token) provider.
///
/// Mirrors [`start_google_oauth_flow`] but uses the Slack authorize URL
/// (`user_scope`) + [`SlackPersonalOAuthCallbackState`] and the Slack client
/// config. Needed because `extension_oauth_start_handler` otherwise routes
/// every non-Google provider to the DCR path, which `slack_personal` is not.
#[cfg(feature = "slack-v2-host-beta")]
async fn start_slack_personal_oauth_flow(
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
    // Model B: the user-installable Slack tools extension (`slack`) owns the
    // slack_personal OAuth flow; the bot channel is operator-provisioned infra.
    if requester_extension.as_str() != SLACK_EXTENSION_ID {
        return Err(ProductAuthRouteFailure::invalid_request());
    }
    // The provider id and scope list are compile-time constants: a failure
    // here is an internal invariant bug, not a malformed client request — do
    // not blame the caller with a 400.
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
    let opaque_state_hash = opaque_state_hash(opaque_state.as_str())?;
    let pkce_verifier_secret = SecretString::from(ironclaw_common::pkce::generate_code_verifier());
    let pkce_verifier_hash = pkce_verifier_hash(pkce_verifier_secret.expose_secret())?;
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

    let flow = run_with_backend_timeout(state.product_auth.start_setup_oauth_flow(
        RebornOAuthStartFlowRequest {
            flow_id: Some(flow_id),
            scope: scope.clone(),
            provider: provider.clone(),
            authorization_url: authorization_url.clone(),
            opaque_state_hash: opaque_state_hash.clone(),
            pkce_verifier_hash,
            update_binding,
            expires_at: request.expires_at,
        },
    ))
    .await?;
    state.store_pkce_verifier(flow.id, pkce_verifier_secret, flow.expires_at)?;

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

async fn start_google_oauth_flow(
    state: ProductAuthRouteState,
    caller: WebUiAuthenticatedCaller,
    request: GoogleOAuthStartRequest,
    requester_extension: Option<ExtensionId>,
    require_invocation_id: bool,
) -> Result<Json<ProductOAuthStartResponse>, ProductAuthRouteFailure> {
    let now = Utc::now();
    if request.expires_at <= now
        || request.expires_at > now + ChronoDuration::seconds(PRODUCT_AUTH_FLOW_MAX_TTL_SECONDS)
    {
        return Err(ProductAuthRouteFailure::invalid_request());
    }

    let config = state.google_oauth_config()?;
    let provider = AuthProviderId::new(GOOGLE_PROVIDER_ID)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let account_label = CredentialAccountLabel::new(request.account_label)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let requested_scopes =
        parse_google_requested_scopes(&request.scopes).map_err(ProductAuthRouteFailure::from)?;
    let fields = ScopeFields {
        session_id: request.session_id,
        thread_id: request.thread_id,
        invocation_id: request.invocation_id,
    };
    let scope = if require_invocation_id {
        scope_from_authenticated_caller_parts_requiring_invocation(&caller, &fields)?
    } else {
        scope_from_authenticated_caller_parts(&caller, &fields)?
    };
    let flow_id = AuthFlowId::new();
    let update_binding = scoped_update_binding_for_requester(
        &state,
        scope.clone(),
        provider.clone(),
        requester_extension.as_ref(),
    )
    .await?;
    let opaque_state = OAuthCallbackState::new(
        OAuthCallbackStateKind::GOOGLE,
        flow_id,
        scope.clone(),
        account_label,
        requested_scopes.clone(),
    )
    .map_err(ProductAuthRouteFailure::from)?
    .encode()
    .map_err(ProductAuthRouteFailure::from)?;
    let opaque_state_hash = opaque_state_hash(opaque_state.as_str())?;
    let pkce_verifier_secret = SecretString::from(ironclaw_common::pkce::generate_code_verifier());
    let pkce_verifier_hash = pkce_verifier_hash(pkce_verifier_secret.expose_secret())?;
    let pkce_secret = PkceVerifierSecret::new(pkce_verifier_secret.clone())
        .map_err(ProductAuthRouteFailure::from)?;
    let pkce_challenge = pkce_s256_challenge(&pkce_secret);
    let authorization_url = build_google_authorization_url(
        config.client_id().as_str(),
        config.redirect_uri().as_str(),
        opaque_state.as_str(),
        &pkce_challenge,
        &requested_scopes,
        config.hosted_domain_hint(),
    )
    .map_err(ProductAuthRouteFailure::from)?;

    let flow = run_with_backend_timeout(state.product_auth.start_setup_oauth_flow(
        RebornOAuthStartFlowRequest {
            flow_id: Some(flow_id),
            scope: scope.clone(),
            provider: provider.clone(),
            authorization_url: authorization_url.clone(),
            opaque_state_hash: opaque_state_hash.clone(),
            pkce_verifier_hash,
            update_binding,
            expires_at: request.expires_at,
        },
    ))
    .await?;
    state.store_pkce_verifier(flow.id, pkce_verifier_secret, flow.expires_at)?;

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

pub(super) async fn oauth_callback_handler(
    State(state): State<ProductAuthRouteState>,
    Path(flow_id): Path<String>,
    RawQuery(raw_query): RawQuery,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ProductAuthRouteFailure> {
    validate_callback_raw_query(raw_query.as_deref())?;
    let query = axum::extract::Query::<OAuthCallbackQuery>::try_from_uri(&uri)
        .map_err(|_| ProductAuthRouteFailure::malformed_callback())?
        .0;
    validate_callback_query_fields(&query)?;

    let flow_id = AuthFlowId::from_uuid(
        Uuid::parse_str(&flow_id).map_err(|_| ProductAuthRouteFailure::malformed_callback())?,
    );
    let state_value = query
        .state
        .as_ref()
        .ok_or_else(ProductAuthRouteFailure::malformed_callback)?;
    let decoded_state = dcr_callback_state_from_oauth_state(state_value.as_str())?;
    if let Some(decoded) = &decoded_state
        && decoded.flow_id() != flow_id
    {
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    let scope = decoded_state
        .as_ref()
        .map(|decoded| decoded.scope().clone())
        .map(Ok)
        .unwrap_or_else(|| scope_from_callback_query(&state, &query))?;
    let state_hash = opaque_state_hash(state_value.as_str())?;

    let flow_provider = if is_authorized_callback_candidate(&query, decoded_state.as_ref()) {
        Some(
            run_with_backend_timeout(
                state
                    .product_auth
                    .ensure_oauth_callback_flow_known(&scope, flow_id),
            )
            .await?,
        )
    } else {
        None
    };
    let outcome = callback_outcome_from_query(
        &state,
        flow_id,
        &scope,
        flow_provider.as_ref(),
        decoded_state.as_ref(),
        &query,
    )
    .await?;

    let response = match run_with_backend_timeout(state.product_auth.handle_oauth_callback(
        RebornOAuthCallbackRequest {
            scope,
            flow_id,
            opaque_state_hash: state_hash,
            outcome,
        },
    ))
    .await
    {
        Ok(response) => {
            state.remove_pkce_verifier(flow_id);
            response
        }
        Err(error) => {
            if should_forget_pkce_verifier(error.body.code) {
                state.remove_pkce_verifier(flow_id);
            }
            return Err(error);
        }
    };

    Ok(oauth_callback_response(&headers, response))
}

/// How a provider's granted scopes are resolved at the callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallbackScopeResolution {
    /// Validate that the provider-echoed `scope` query includes every requested
    /// scope, then submit the requested scopes (Google).
    ValidateEchoedIncludesRequested,
    /// The provider does not echo granted scopes on the redirect; submit the
    /// requested scopes from the encoded state directly (Slack personal — the
    /// granted scopes arrive later in the token response under `authed_user`).
    #[cfg(feature = "slack-v2-host-beta")]
    RequestedOnly,
}

/// Provider-specific parameters that drive the single [`oauth_provider_callback_handler`].
///
/// Google and Slack become two descriptor values instead of two near-identical
/// hand-written callback handlers.
struct OAuthCallbackDescriptor {
    /// Wire prefix + scope policy used to decode the callback `state`.
    state_kind: OAuthCallbackStateKind,
    /// Reborn provider id submitted with the token exchange.
    provider_id: &'static str,
    /// How granted scopes are resolved at the callback.
    scope_resolution: CallbackScopeResolution,
    /// Optional post-exchange provider-identity check (Slack binds the
    /// `authed_user` identity; Google has none). Built after the callback scope
    /// is decoded so it can capture the scope.
    identity_hook:
        fn(&ProductAuthRouteState, &AuthProductScope) -> Option<OAuthProviderIdentityCheck>,
}

/// No post-exchange identity check (Google).
fn no_identity_hook(
    _state: &ProductAuthRouteState,
    _callback_scope: &AuthProductScope,
) -> Option<OAuthProviderIdentityCheck> {
    None
}

static GOOGLE_CALLBACK_DESCRIPTOR: OAuthCallbackDescriptor = OAuthCallbackDescriptor {
    state_kind: OAuthCallbackStateKind::GOOGLE,
    provider_id: GOOGLE_PROVIDER_ID,
    scope_resolution: CallbackScopeResolution::ValidateEchoedIncludesRequested,
    identity_hook: no_identity_hook,
};

#[cfg(feature = "slack-v2-host-beta")]
static SLACK_PERSONAL_CALLBACK_DESCRIPTOR: OAuthCallbackDescriptor = OAuthCallbackDescriptor {
    state_kind: OAuthCallbackStateKind::SLACK_PERSONAL,
    provider_id: SLACK_PERSONAL_PROVIDER_ID,
    scope_resolution: CallbackScopeResolution::RequestedOnly,
    identity_hook: slack_personal_identity_hook,
};

enum CallbackScopeOutcome {
    Scopes(Vec<ProviderScope>),
    ProviderDenied,
}

fn resolve_callback_scopes(
    resolution: CallbackScopeResolution,
    requested_scopes: &[ProviderScope],
    query_scopes: Option<&str>,
) -> Result<CallbackScopeOutcome, ProductAuthRouteFailure> {
    match resolution {
        #[cfg(feature = "slack-v2-host-beta")]
        CallbackScopeResolution::RequestedOnly => {
            Ok(CallbackScopeOutcome::Scopes(requested_scopes.to_vec()))
        }
        CallbackScopeResolution::ValidateEchoedIncludesRequested => {
            match parse_google_callback_scopes(query_scopes) {
                Ok(Some(callback_scopes)) => {
                    if validate_google_callback_includes_requested_scopes(
                        &callback_scopes,
                        requested_scopes,
                    )
                    .is_err()
                    {
                        Ok(CallbackScopeOutcome::ProviderDenied)
                    } else {
                        Ok(CallbackScopeOutcome::Scopes(requested_scopes.to_vec()))
                    }
                }
                Ok(None) => Ok(CallbackScopeOutcome::Scopes(requested_scopes.to_vec())),
                Err(error) => Err(ProductAuthRouteFailure::from(error)),
            }
        }
    }
}

/// Google product-auth OAuth callback. Thin wrapper over the shared
/// [`oauth_provider_callback_handler`] with the Google descriptor.
pub(super) async fn google_oauth_callback_handler(
    State(state): State<ProductAuthRouteState>,
    RawQuery(raw_query): RawQuery,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ProductAuthRouteFailure> {
    oauth_provider_callback_handler(state, &GOOGLE_CALLBACK_DESCRIPTOR, raw_query, uri, headers)
        .await
}

/// Slack personal (user-token) OAuth callback. Thin wrapper over the shared
/// [`oauth_provider_callback_handler`] with the Slack descriptor, which adds the
/// `authed_user`-identity binding hook.
#[cfg(feature = "slack-v2-host-beta")]
pub(super) async fn slack_personal_oauth_callback_handler(
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

/// One OAuth callback handler for every static-callback-URL provider (Google,
/// Slack personal). The provider differences — state decode prefix/policy,
/// provider id, scope resolution, and post-exchange identity binding — are
/// carried by `descriptor`.
///
/// Safety-preserving invariants (identical for both providers): the raw `state`
/// is hashed once and claimed through `AuthFlowManager` (CSRF/state-hash +
/// single-use/replay), the PKCE verifier is resolved from the process-local
/// cache then the durable gate store, provider tokens are exchanged only after
/// the flow is claimed, and the callback tenant must match the route tenant
/// before any exchange.
async fn oauth_provider_callback_handler(
    state: ProductAuthRouteState,
    descriptor: &OAuthCallbackDescriptor,
    raw_query: Option<String>,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ProductAuthRouteFailure> {
    // Browser popups (Accept: text/html) must never see a bare JSON route
    // failure: render the failure page instead, which emits the cross-window
    // "failed" completion signal (with the flow id once the state decoded) and
    // closes the popup so the parent surface can show a retryable error.
    let mut known_flow_id = None;
    match oauth_provider_callback_attempt(
        state,
        descriptor,
        raw_query,
        uri,
        &headers,
        &mut known_flow_id,
    )
    .await
    {
        Ok(response) => Ok(response),
        Err(error) if wants_oauth_callback_html(&headers) => Ok(oauth_callback_failure_html(
            error.status,
            &error.body,
            known_flow_id,
        )),
        Err(error) => Err(error),
    }
}

async fn oauth_provider_callback_attempt(
    state: ProductAuthRouteState,
    descriptor: &OAuthCallbackDescriptor,
    raw_query: Option<String>,
    uri: Uri,
    headers: &HeaderMap,
    known_flow_id: &mut Option<AuthFlowId>,
) -> Result<Response, ProductAuthRouteFailure> {
    validate_callback_raw_query(raw_query.as_deref())?;
    let query = axum::extract::Query::<GoogleOAuthCallbackQuery>::try_from_uri(&uri)
        .map_err(|_| ProductAuthRouteFailure::malformed_callback())?
        .0;
    validate_google_callback_query_fields(&query)?;
    let state_value = query
        .state
        .as_ref()
        .ok_or_else(ProductAuthRouteFailure::malformed_callback)?;
    let state_hash = opaque_state_hash(state_value.as_str())?;
    let callback_state = OAuthCallbackState::decode(descriptor.state_kind, state_value.as_str())
        .map_err(ProductAuthRouteFailure::from)?;
    let flow_id = callback_state.flow_id();
    *known_flow_id = Some(flow_id);
    let callback_scope = callback_state.scope();
    // Reject a callback state minted for another tenant before any provider
    // exchange. Applied to BOTH providers: unification strengthens Google, which
    // previously lacked this check (Slack already had it).
    if callback_scope.resource.tenant_id != state.tenant_id {
        return Err(ProductAuthRouteFailure::new(
            StatusCode::FORBIDDEN,
            AuthErrorCode::CrossScopeDenied,
        ));
    }

    if query
        .error
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        let response = run_with_backend_timeout(state.product_auth.handle_oauth_callback(
            RebornOAuthCallbackRequest {
                scope: callback_scope.clone(),
                flow_id,
                opaque_state_hash: state_hash.clone(),
                outcome: RebornOAuthCallbackOutcome::ProviderDenied,
            },
        ))
        .await;
        state.remove_pkce_verifier(flow_id);
        return oauth_callback_route_result_response(headers, response);
    }

    let provider = match run_with_backend_timeout(
        state
            .product_auth
            .ensure_oauth_callback_flow_known(callback_scope, flow_id),
    )
    .await
    {
        Ok(provider) => provider,
        Err(error) => {
            state.remove_pkce_verifier(flow_id);
            return Err(error);
        }
    };
    let Some(code) = query.code.as_ref() else {
        state.remove_pkce_verifier(flow_id);
        return Err(ProductAuthRouteFailure::malformed_callback());
    };
    let pkce_verifier =
        match pkce_verifier_for_known_callback_flow(&state, callback_scope, &provider, flow_id)
            .await
        {
            Ok(pkce_verifier) => pkce_verifier,
            Err(error) => {
                state.remove_pkce_verifier(flow_id);
                return Err(error);
            }
        };
    let callback_scopes = match resolve_callback_scopes(
        descriptor.scope_resolution,
        callback_state.requested_scopes(),
        query.scopes.as_deref(),
    ) {
        Ok(CallbackScopeOutcome::Scopes(scopes)) => scopes,
        Ok(CallbackScopeOutcome::ProviderDenied) => {
            state.remove_pkce_verifier(flow_id);
            let response = run_with_backend_timeout(state.product_auth.handle_oauth_callback(
                RebornOAuthCallbackRequest {
                    scope: callback_scope.clone(),
                    flow_id,
                    opaque_state_hash: state_hash.clone(),
                    outcome: RebornOAuthCallbackOutcome::ProviderDenied,
                },
            ))
            .await;
            return oauth_callback_route_result_response(headers, response);
        }
        Err(error) => {
            state.remove_pkce_verifier(flow_id);
            return Err(error);
        }
    };
    let authorization_code_hash = authorization_code_hash(code.expose_secret())?;
    let pkce_verifier_hash = pkce_verifier_hash(pkce_verifier.expose_secret())?;

    let callback_request = RebornOAuthCallbackRequest {
        scope: callback_scope.clone(),
        flow_id,
        opaque_state_hash: state_hash.clone(),
        outcome: RebornOAuthCallbackOutcome::Authorized {
            provider_request: OAuthProviderCallbackRequest {
                provider: AuthProviderId::new(descriptor.provider_id)
                    .map_err(|_| ProductAuthRouteFailure::malformed_callback())?,
                account_label: callback_state.account_label().clone(),
                authorization_code: OAuthAuthorizationCode::new(code.clone_secret())
                    .map_err(ProductAuthRouteFailure::from)?,
                authorization_code_hash,
                pkce_verifier: PkceVerifierSecret::new(pkce_verifier)
                    .map_err(ProductAuthRouteFailure::from)?,
                pkce_verifier_hash,
                scopes: callback_scopes,
            },
        },
    };
    let identity_check = (descriptor.identity_hook)(&state, callback_scope);
    let response = match run_with_backend_timeout(
        state
            .product_auth
            .handle_oauth_callback_with_optional_provider_identity_check(
                callback_request,
                identity_check,
            ),
    )
    .await
    {
        Ok(response) => {
            state.remove_pkce_verifier(flow_id);
            response
        }
        Err(error) => {
            if should_forget_pkce_verifier(error.body.code) {
                state.remove_pkce_verifier(flow_id);
            }
            return Err(error);
        }
    };

    Ok(oauth_callback_response(headers, response))
}

/// Slack post-exchange identity hook: binds the exchanged `authed_user` identity
/// to the authenticated Reborn user. Captures a clone of the route state and the
/// decoded callback scope.
#[cfg(feature = "slack-v2-host-beta")]
fn slack_personal_identity_hook(
    state: &ProductAuthRouteState,
    callback_scope: &AuthProductScope,
) -> Option<OAuthProviderIdentityCheck> {
    let state = state.clone();
    let callback_scope = callback_scope.clone();
    Some(Box::new(
        move |provider_identity: Option<OAuthProviderIdentity>| {
            Box::pin(async move {
                bind_slack_personal_oauth_identity_for_callback(
                    &state,
                    &callback_scope,
                    provider_identity.as_ref(),
                )
                .await
                .map(Some)
            }) as OAuthProviderIdentityCheckFuture
        },
    ))
}

#[cfg(feature = "slack-v2-host-beta")]
async fn bind_slack_personal_oauth_identity_for_callback(
    state: &ProductAuthRouteState,
    callback_scope: &AuthProductScope,
    provider_identity: Option<&OAuthProviderIdentity>,
) -> Result<crate::product_auth::api::auth::OAuthProviderIdentityBindingRollback, AuthProductError>
{
    // Fail closed: the Slack callback descriptor is only mounted when Slack
    // personal OAuth is wired, so a missing binding config here is a
    // composition bug. Silently skipping would store a live xoxp token with no
    // identity binding — the UI would show "not connected" forever while the
    // credential exists and is selectable.
    let Some(config) = state.slack_personal_oauth_binding_config() else {
        tracing::warn!(
            "Slack personal OAuth callback reached without a binding config; failing closed"
        );
        return Err(AuthProductError::BackendUnavailable);
    };
    let identity = provider_identity.ok_or(AuthProductError::MalformedCallback)?;
    let connection_scope = config
        .connection_scope_resolver
        .resolve_personal_connection_scope()
        .await
        .map_err(|error| {
            tracing::warn!(
                %error,
                "Slack personal OAuth binding connection scope resolver failed"
            );
            AuthProductError::BackendUnavailable
        })?
        .ok_or(AuthProductError::BackendUnavailable)?;
    let team_id = identity
        .team_id
        .as_ref()
        .ok_or(AuthProductError::MalformedCallback)?;
    if team_id.as_str() != connection_scope.team_id.as_str() {
        return Err(AuthProductError::MalformedCallback);
    }
    let api_app_id = identity
        .app_id
        .as_ref()
        .ok_or(AuthProductError::MalformedCallback)?;
    let enterprise_id = identity
        .enterprise_id
        .as_ref()
        .map(|value| SlackEnterpriseId::new(value.clone()));

    // Computed before the request takes ownership of the installation id so
    // the rollback can target exactly the binding this callback writes.
    let bound_provider_user_id =
        crate::slack::slack_actor_identity::slack_user_identity_provider_user_id(
            &connection_scope.installation_id,
            identity.subject.as_str(),
        );
    config
        .binding_service
        .bind_personal_user(
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
        )
        .await
        .map_err(slack_personal_user_binding_auth_error)?;

    let rollback_store = Arc::clone(&config.binding_rollback_store);
    let rollback_user_id = callback_scope.resource.user_id.clone();
    Ok(Box::pin(async move {
        // Passing the full provider_user_id as the prefix confines the delete
        // to the binding this exact callback wrote. Best-effort by contract:
        // a rollback failure only errs toward "shows connected without a
        // credential", which Disconnect already repairs.
        if let Err(error) = rollback_store
            .delete_user_identity_bindings_for_user(
                crate::slack::slack_actor_identity::SLACK_IDENTITY_PROVIDER,
                &rollback_user_id,
                Some(bound_provider_user_id.as_str()),
            )
            .await
        {
            tracing::warn!(
                %error,
                "failed to roll back Slack identity binding after OAuth completion failure"
            );
        }
    }))
}

#[cfg(feature = "slack-v2-host-beta")]
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
        SlackPersonalUserBindingError::BindingStore(_) => AuthProductError::BackendUnavailable,
    }
}

// Formats the success shape only; `Err` propagates so the handler wrapper
// renders the (signal-emitting, flow-id-aware) failure page for browser popups.
fn oauth_callback_route_result_response(
    headers: &HeaderMap,
    response: Result<RebornOAuthCallbackResponse, ProductAuthRouteFailure>,
) -> Result<Response, ProductAuthRouteFailure> {
    response.map(|response| oauth_callback_response(headers, response))
}

fn validate_google_callback_includes_requested_scopes(
    callback_scopes: &[ProviderScope],
    requested_scopes: &[ProviderScope],
) -> Result<(), ProductAuthRouteFailure> {
    if callback_scopes.is_empty()
        || !requested_scopes
            .iter()
            .all(|requested| callback_scopes.iter().any(|scope| scope == requested))
    {
        return Err(ProductAuthRouteFailure::new(
            StatusCode::BAD_REQUEST,
            AuthErrorCode::ProviderDenied,
        ));
    }
    Ok(())
}

fn oauth_callback_response(headers: &HeaderMap, response: RebornOAuthCallbackResponse) -> Response {
    if wants_oauth_callback_html(headers) {
        return oauth_callback_completion_html(&response);
    }
    Json(response).into_response()
}

fn wants_oauth_callback_html(headers: &HeaderMap) -> bool {
    let Some(accept) = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let accepts_html = accept
        .split(',')
        .any(|part| part.trim_start().starts_with("text/html"));
    let accepts_json = accept
        .split(',')
        .any(|part| part.trim_start().starts_with("application/json"));
    accepts_html && !accepts_json
}

const OAUTH_CALLBACK_SIGNAL_CHANNEL: &str = "ironclaw-product-auth";
const OAUTH_CALLBACK_SIGNAL_STORAGE_KEY: &str = "ironclaw:product-auth:oauth-complete";
const OAUTH_CALLBACK_SIGNAL_MESSAGE_TYPE: &str = "ironclaw:product-auth:oauth-complete";

/// Inline script that hands a completion/failure payload to the opener via
/// BroadcastChannel + localStorage (both same-origin best-effort) and closes
/// the popup. Shared by the success and failure callback pages so the two
/// cannot drift on the signal contract the WebUI listens for.
fn oauth_callback_signal_script(payload: &serde_json::Value) -> String {
    // serde_json escapes quotes/control chars but not `<`: a value containing
    // `</script>` would otherwise terminate the element mid-string (HTML
    // parsing ignores JS string context). All payload values are server-minted
    // today; this keeps that assumption non-load-bearing.
    let payload = payload.to_string().replace('<', "\\u003c");
    format!(
        r#"  <script>
    (() => {{
      const payload = {payload};
      try {{
        new BroadcastChannel("{OAUTH_CALLBACK_SIGNAL_CHANNEL}").postMessage(payload);
      }} catch (_err) {{}}
      try {{
        localStorage.setItem(
          "{OAUTH_CALLBACK_SIGNAL_STORAGE_KEY}",
          JSON.stringify({{ ...payload, completedAt: Date.now() }})
        );
      }} catch (_err) {{}}
      window.close();
    }})();
  </script>"#
    )
}

fn oauth_callback_completion_html(response: &RebornOAuthCallbackResponse) -> Response {
    let (title, message) = match response.status {
        AuthFlowStatus::Completed => (
            "Authorization complete",
            "Authorization complete. You can close this window.",
        ),
        AuthFlowStatus::Failed => (
            "Authorization failed",
            "Authorization failed. No permissions were selected, or authorization was denied. Please retry authorization and select the requested permissions.",
        ),
        _ => (
            "Authorization failed",
            "Authorization did not complete. Please return to Reborn and retry authorization.",
        ),
    };

    let script = oauth_callback_signal_script(&json!({
        "type": OAUTH_CALLBACK_SIGNAL_MESSAGE_TYPE,
        "flowId": response.flow_id,
        "status": response.status,
        "continuation": response.continuation,
    }));
    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>{title}</title>
</head>
<body>
  <p>{message}</p>
{script}
</body>
</html>"#
    );
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
}

fn oauth_callback_failure_html(
    status: StatusCode,
    error: &RebornOAuthCallbackError,
    flow_id: Option<AuthFlowId>,
) -> Response {
    let message = match error.code {
        AuthErrorCode::ProviderDenied => {
            "Authorization failed. No permissions were selected, or authorization was denied. Please retry authorization and select the requested permissions."
        }
        AuthErrorCode::MalformedCallback => {
            "Authorization failed. Please retry authorization from Reborn."
        }
        _ => "Authorization failed. Please return to Reborn and retry authorization.",
    };
    let script = oauth_callback_signal_script(&json!({
        "type": OAUTH_CALLBACK_SIGNAL_MESSAGE_TYPE,
        "flowId": flow_id,
        "status": ironclaw_auth::AuthFlowStatus::Failed,
        "errorCode": error.code,
    }));
    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Authorization failed</title>
</head>
<body>
  <p>{message}</p>
{script}
</body>
</html>"#
    );
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

pub(super) async fn callback_outcome_from_query(
    state: &ProductAuthRouteState,
    flow_id: AuthFlowId,
    scope: &AuthProductScope,
    flow_provider: Option<&AuthProviderId>,
    callback_state: Option<&DcrOAuthCallbackState>,
    query: &OAuthCallbackQuery,
) -> Result<RebornOAuthCallbackOutcome, ProductAuthRouteFailure> {
    if query
        .error
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return Ok(RebornOAuthCallbackOutcome::ProviderDenied);
    }

    let provider = match query.provider.as_deref() {
        Some(provider) => AuthProviderId::new(provider.to_string())
            .map_err(|_| ProductAuthRouteFailure::malformed_callback())?,
        None => callback_state
            .map(|state| state.provider().clone())
            .ok_or_else(ProductAuthRouteFailure::malformed_callback)?,
    };
    if flow_provider.is_some_and(|known_provider| known_provider != &provider) {
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    let account_label = match query.account_label.as_deref() {
        Some(account_label) => CredentialAccountLabel::new(account_label.to_string())
            .map_err(|_| ProductAuthRouteFailure::malformed_callback())?,
        None => callback_state
            .map(|state| state.account_label().clone())
            .ok_or_else(ProductAuthRouteFailure::malformed_callback)?,
    };
    let code = query
        .code
        .as_ref()
        .ok_or_else(ProductAuthRouteFailure::malformed_callback)?;
    let pkce_verifier = pkce_verifier_for_known_callback_flow(
        state,
        scope,
        flow_provider.unwrap_or(&provider),
        flow_id,
    )
    .await?;
    let scopes = match query.scopes.as_deref() {
        Some(raw) => parse_provider_scopes(Some(raw))?,
        None => callback_state
            .map(|state| state.requested_scopes().to_vec())
            .unwrap_or_default(),
    };
    let authorization_code_hash = authorization_code_hash(code.expose_secret())?;
    let pkce_verifier_hash = pkce_verifier_hash(pkce_verifier.expose_secret())?;

    Ok(RebornOAuthCallbackOutcome::Authorized {
        provider_request: OAuthProviderCallbackRequest {
            provider,
            account_label,
            authorization_code: OAuthAuthorizationCode::new(code.clone_secret())
                .map_err(ProductAuthRouteFailure::from)?,
            authorization_code_hash,
            pkce_verifier: PkceVerifierSecret::new(pkce_verifier)
                .map_err(ProductAuthRouteFailure::from)?,
            pkce_verifier_hash,
            scopes,
        },
    })
}

async fn pkce_verifier_for_known_callback_flow(
    state: &ProductAuthRouteState,
    scope: &AuthProductScope,
    provider: &AuthProviderId,
    flow_id: AuthFlowId,
) -> Result<SecretString, ProductAuthRouteFailure> {
    let cache_error = match state.pkce_verifier_for_callback(flow_id) {
        Ok(verifier) => return Ok(verifier),
        Err(error) => error,
    };
    run_with_backend_timeout(
        state
            .product_auth
            .oauth_pkce_verifier_for_flow(scope, provider, flow_id),
    )
    .await?
    .ok_or(cache_error)
}

fn validate_google_callback_query_fields(
    query: &GoogleOAuthCallbackQuery,
) -> Result<(), ProductAuthRouteFailure> {
    validate_optional_callback_field(
        query.error.as_deref(),
        OAUTH_CALLBACK_FIELD_MAX_BYTES,
        false,
    )?;
    validate_optional_callback_field(
        query.scopes.as_deref(),
        OAUTH_CALLBACK_SCOPES_MAX_BYTES,
        true,
    )?;
    Ok(())
}

pub(super) fn is_authorized_callback_candidate(
    query: &OAuthCallbackQuery,
    callback_state: Option<&DcrOAuthCallbackState>,
) -> bool {
    query.error.as_deref().is_none_or(|value| value.is_empty())
        && (query.provider.is_some() || callback_state.is_some())
        && (query.account_label.is_some() || callback_state.is_some())
        && query.code.is_some()
}

pub(super) fn should_forget_pkce_verifier(code: AuthErrorCode) -> bool {
    matches!(
        code,
        AuthErrorCode::ProviderDenied
            | AuthErrorCode::Canceled
            | AuthErrorCode::FlowAlreadyTerminal
            | AuthErrorCode::TokenExchangeFailed
            | AuthErrorCode::RefreshFailed
            | AuthErrorCode::CredentialMissing
            | AuthErrorCode::AccountSelectionRequired
    )
}

fn dcr_callback_state_from_oauth_state(
    state: &str,
) -> Result<Option<DcrOAuthCallbackState>, ProductAuthRouteFailure> {
    if !DcrOAuthCallbackState::has_prefix(state) {
        return Ok(None);
    }
    DcrOAuthCallbackState::decode(state)
        .map(Some)
        .map_err(ProductAuthRouteFailure::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuthChallengeProvider;
    use crate::RebornAuthContinuationDispatcher;
    use crate::input::OAuthClientConfig;
    use crate::product_auth::oauth::oauth_gate::{
        GoogleOAuthGateProvider, OAuthGateFlowDriver, OAuthGateProviderRegistry,
    };
    use async_trait::async_trait;
    use axum::body::to_bytes;
    use ironclaw_auth::{
        AuthProviderClient, CredentialAccountRecordSource, GOOGLE_CALENDAR_READONLY_SCOPE,
        InMemoryAuthProductServices, OAuthProviderExchange, OAuthProviderExchangeContext,
        OAuthProviderIdentity, OAuthProviderRefresh, OAuthProviderRefreshRequest,
    };
    use ironclaw_host_api::{
        RuntimeCredentialAccountProviderId, RuntimeCredentialAuthRequirement, SecretHandle,
    };
    #[cfg(feature = "slack-v2-host-beta")]
    use ironclaw_product_adapters::AdapterInstallationId;
    use ironclaw_secrets::{InMemorySecretStore, SecretStore};
    use ironclaw_turns::{TurnRunId, TurnScope};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[cfg(feature = "slack-v2-host-beta")]
    use crate::slack::slack_host_beta::{
        SlackPersonalConnectionScope, StaticSlackPersonalConnectionScopeResolver,
    };
    #[cfg(feature = "slack-v2-host-beta")]
    use crate::slack::slack_personal_binding::{
        RebornUserIdentityBinding, RebornUserIdentityBindingError, RebornUserIdentityBindingStore,
        SlackPersonalBindingInstallation, SlackPersonalUserBindingService,
    };
    #[cfg(feature = "slack-v2-host-beta")]
    use crate::slack::slack_serve::{SlackInstallationSelector, SlackTeamId};
    #[cfg(feature = "slack-v2-host-beta")]
    use crate::slack::slack_setup::{
        SlackInstallationSetup, SlackInstallationSetupStore, SlackInstallationSetupUpdate,
        SlackPersonalSetupServiceSlot, SlackSetupError, SlackSetupService,
    };

    #[cfg(feature = "slack-v2-host-beta")]
    #[derive(Debug, Default)]
    struct MemorySlackSetupStore {
        setup: Mutex<Option<SlackInstallationSetup>>,
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[async_trait]
    impl SlackInstallationSetupStore for MemorySlackSetupStore {
        async fn get_slack_installation_setup(
            &self,
        ) -> Result<Option<SlackInstallationSetup>, SlackSetupError> {
            Ok(self.setup.lock().expect("setup lock").clone())
        }

        async fn put_slack_installation_setup(
            &self,
            setup: &SlackInstallationSetup,
        ) -> Result<(), SlackSetupError> {
            *self.setup.lock().expect("setup lock") = Some(setup.clone());
            Ok(())
        }

        async fn delete_slack_installation_setup(&self) -> Result<(), SlackSetupError> {
            *self.setup.lock().expect("setup lock") = None;
            Ok(())
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    async fn slack_personal_oauth_test_slot() -> SlackPersonalSetupServiceSlot {
        let redirect_uri = ironclaw_auth::OAuthRedirectUri::new(
            "http://127.0.0.1:3000/api/reborn/product-auth/oauth/slack_personal/callback",
        )
        .expect("slack oauth redirect uri");
        let slot = SlackPersonalSetupServiceSlot::new(redirect_uri);
        let service = Arc::new(SlackSetupService::new(
            TenantId::new("tenant-alpha").expect("tenant"),
            AgentId::new("agent:test").expect("agent"),
            None,
            UserId::new("user:operator").expect("operator"),
            Arc::new(MemorySlackSetupStore::default()),
            Arc::new(InMemorySecretStore::new()),
        ));
        service
            .save(SlackInstallationSetupUpdate {
                installation_id: "install-alpha".to_string(),
                team_id: "T123".to_string(),
                api_app_id: "A123".to_string(),
                user_id: Some("user:operator".to_string()),
                shared_subject_user_id: None,
                bot_token: Some(SecretString::from("xoxb-test")),
                signing_secret: Some(SecretString::from("slack-signing-test")),
                oauth_client_id: Some("slack-client".to_string()),
                oauth_client_secret: Some(SecretString::from("slack-client-secret")),
            })
            .await
            .expect("seed slack setup");
        slot.fill(service);
        slot
    }

    #[tokio::test]
    async fn google_oauth_callback_uses_gate_pkce_store_when_route_cache_misses() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let secret_store = Arc::new(InMemorySecretStore::new());
        let secret_store_for_provider: Arc<dyn SecretStore> = secret_store.clone();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let google_gate = Arc::new(OAuthGateFlowDriver::new(
            Arc::new(GoogleOAuthGateProvider::new(
                OAuthClientConfig::new(
                    "google-client.apps.googleusercontent.com",
                    "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
                    None,
                )
                .expect("google oauth client"),
            )),
            secret_store_for_provider,
        ));
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), dispatcher.clone())
                .with_flow_record_source(shared)
                .with_oauth_gate_registry(Arc::new(OAuthGateProviderRegistry::new(vec![
                    google_gate,
                ]))),
        );
        let state = ProductAuthRouteState::new(
            product_auth.clone(),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
        let turn_scope = TurnScope::new(
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
            ThreadId::new("thread-alpha").expect("thread"),
        );
        let owner_user_id = UserId::new("user-alpha").expect("user");
        let run_id = TurnRunId::new();
        let gate_ref = "gate:google-auth";
        let requirements = vec![RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("google").expect("provider"),
            setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
                scopes: vec![GOOGLE_CALENDAR_READONLY_SCOPE.to_string()],
            },
            requester_extension: ExtensionId::new("google-calendar").expect("extension"),
            provider_scopes: vec![GOOGLE_CALENDAR_READONLY_SCOPE.to_string()],
        }];

        let challenge = product_auth
            .challenge_for_gate(&turn_scope, &owner_user_id, run_id, gate_ref, &requirements)
            .await
            .expect("challenge lookup")
            .expect("google oauth challenge");
        let authorization_url = challenge.authorization_url.expect("authorization url");
        let state_value = Url::parse(authorization_url.as_str())
            .expect("authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("oauth state");
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let encoded_scope =
            url::form_urlencoded::byte_serialize(GOOGLE_CALENDAR_READONLY_SCOPE.as_bytes())
                .collect::<String>();
        let uri = format!(
            "{GOOGLE_OAUTH_CALLBACK_PATH}?state={encoded_state}&code=google-auth-code&scope={encoded_scope}"
        )
        .parse::<Uri>()
        .expect("callback uri");

        let response = google_oauth_callback_handler(
            State(state),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            HeaderMap::new(),
        )
        .await
        .expect("google callback");

        assert_eq!(response.status(), StatusCode::OK);
        let events = dispatcher.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].continuation,
            AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(run_id.to_string()).expect("run ref"),
                gate_ref: AuthGateRef::new(gate_ref).expect("gate ref"),
            }
        );
    }

    #[tokio::test]
    async fn google_oauth_callback_with_empty_scope_returns_html_failure_page() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let secret_store = Arc::new(InMemorySecretStore::new());
        let secret_store_for_provider: Arc<dyn SecretStore> = secret_store.clone();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let google_gate = Arc::new(OAuthGateFlowDriver::new(
            Arc::new(GoogleOAuthGateProvider::new(
                OAuthClientConfig::new(
                    "google-client.apps.googleusercontent.com",
                    "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
                    None,
                )
                .expect("google oauth client"),
            )),
            secret_store_for_provider,
        ));
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), dispatcher)
                .with_flow_record_source(shared)
                .with_oauth_gate_registry(Arc::new(OAuthGateProviderRegistry::new(vec![
                    google_gate,
                ]))),
        );
        let state = ProductAuthRouteState::new(
            product_auth.clone(),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
        let turn_scope = TurnScope::new(
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
            ThreadId::new("thread-alpha").expect("thread"),
        );
        let owner_user_id = UserId::new("user-alpha").expect("user");
        let requirements = vec![RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("google").expect("provider"),
            setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
                scopes: vec![GOOGLE_CALENDAR_READONLY_SCOPE.to_string()],
            },
            requester_extension: ExtensionId::new("gmail").expect("extension"),
            provider_scopes: vec![GOOGLE_CALENDAR_READONLY_SCOPE.to_string()],
        }];

        let challenge = product_auth
            .challenge_for_gate(
                &turn_scope,
                &owner_user_id,
                TurnRunId::new(),
                "gate:gmail-auth",
                &requirements,
            )
            .await
            .expect("challenge lookup")
            .expect("google oauth challenge");
        let authorization_url = challenge.authorization_url.expect("authorization url");
        let state_value = Url::parse(authorization_url.as_str())
            .expect("authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("oauth state");
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let uri = format!(
            "{GOOGLE_OAUTH_CALLBACK_PATH}?state={encoded_state}&code=google-auth-code&scope="
        )
        .parse::<Uri>()
        .expect("callback uri");
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            "text/html,application/xhtml+xml"
                .parse()
                .expect("accept header"),
        );

        let response = google_oauth_callback_handler(
            State(state),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            headers,
        )
        .await
        .expect("google callback renders html failure");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&"text/html; charset=utf-8".parse().expect("content type"))
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let body = String::from_utf8(body.to_vec()).expect("utf8 body");
        assert!(body.contains("Authorization failed"));
        assert!(body.contains("No permissions were selected"));
        assert!(!body.contains("malformed_callback"));
        // The failure page must emit the same cross-window completion signal as
        // the success page (status "failed"), and close the popup, so the
        // parent Extensions modal / in-chat card can surface a retryable error
        // instead of spinning until timeout.
        let flow_id = OAuthCallbackState::decode(OAuthCallbackStateKind::GOOGLE, &state_value)
            .expect("decode callback state")
            .flow_id();
        assert!(
            body.contains(r#"new BroadcastChannel("ironclaw-product-auth")"#),
            "failure page must broadcast the completion signal: {body}"
        );
        assert!(
            body.contains(r#""status":"failed""#),
            "failure signal must carry status failed: {body}"
        );
        assert!(
            body.contains(&flow_id.to_string()),
            "failure signal must carry the flow id so only the owning window reacts: {body}"
        );
        assert!(body.contains("window.close()"));
    }

    #[tokio::test]
    async fn oauth_callback_route_failure_renders_html_failure_with_completion_signal() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let secret_store = Arc::new(InMemorySecretStore::new());
        let secret_store_for_provider: Arc<dyn SecretStore> = secret_store.clone();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let google_gate = Arc::new(OAuthGateFlowDriver::new(
            Arc::new(GoogleOAuthGateProvider::new(
                OAuthClientConfig::new(
                    "google-client.apps.googleusercontent.com",
                    "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
                    None,
                )
                .expect("google oauth client"),
            )),
            secret_store_for_provider,
        ));
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), dispatcher)
                .with_flow_record_source(shared)
                .with_oauth_gate_registry(Arc::new(OAuthGateProviderRegistry::new(vec![
                    google_gate,
                ]))),
        );
        // The flow's callback state is minted for tenant-alpha, but the route
        // serves tenant-beta: the post-decode cross-tenant rejection is a route
        // `Err` (not a handled callback outcome), which used to reach the
        // browser popup as a bare JSON failure with no signal and no close.
        let state = ProductAuthRouteState::new(
            product_auth.clone(),
            TenantId::new("tenant-beta").expect("tenant"),
            None,
            None,
        );
        let turn_scope = TurnScope::new(
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
            ThreadId::new("thread-alpha").expect("thread"),
        );
        let owner_user_id = UserId::new("user-alpha").expect("user");
        let requirements = vec![RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("google").expect("provider"),
            setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
                scopes: vec![GOOGLE_CALENDAR_READONLY_SCOPE.to_string()],
            },
            requester_extension: ExtensionId::new("gmail").expect("extension"),
            provider_scopes: vec![GOOGLE_CALENDAR_READONLY_SCOPE.to_string()],
        }];

        let challenge = product_auth
            .challenge_for_gate(
                &turn_scope,
                &owner_user_id,
                TurnRunId::new(),
                "gate:gmail-auth",
                &requirements,
            )
            .await
            .expect("challenge lookup")
            .expect("google oauth challenge");
        let authorization_url = challenge.authorization_url.expect("authorization url");
        let state_value = Url::parse(authorization_url.as_str())
            .expect("authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("oauth state");
        let flow_id = OAuthCallbackState::decode(OAuthCallbackStateKind::GOOGLE, &state_value)
            .expect("decode callback state")
            .flow_id();
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let encoded_scope =
            url::form_urlencoded::byte_serialize(GOOGLE_CALENDAR_READONLY_SCOPE.as_bytes())
                .collect::<String>();
        let uri = format!(
            "{GOOGLE_OAUTH_CALLBACK_PATH}?state={encoded_state}&code=google-auth-code&scope={encoded_scope}"
        )
        .parse::<Uri>()
        .expect("callback uri");
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            "text/html,application/xhtml+xml"
                .parse()
                .expect("accept header"),
        );

        let response = google_oauth_callback_handler(
            State(state),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            headers,
        )
        .await
        .expect("a route failure must render an HTML failure page for a browser popup");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&"text/html; charset=utf-8".parse().expect("content type"))
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let body = String::from_utf8(body.to_vec()).expect("utf8 body");
        assert!(
            body.contains(r#"new BroadcastChannel("ironclaw-product-auth")"#),
            "exchange-failure page must broadcast the completion signal: {body}"
        );
        assert!(body.contains(r#""status":"failed""#));
        assert!(body.contains(&flow_id.to_string()));
        assert!(body.contains("window.close()"));
    }

    #[tokio::test]
    async fn oauth_callback_rejects_dcr_state_with_mismatched_path_flow_id() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let product_auth = Arc::new(RebornProductAuthServices::from_shared(
            shared,
            Arc::new(RecordingDispatcher::default()),
        ));
        let state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
        let state_flow_id = AuthFlowId::new();
        let path_flow_id = AuthFlowId::new();
        let scope = AuthProductScope::new(
            ResourceScope::local_default(
                UserId::new("user-alpha").expect("user"),
                InvocationId::new(),
            )
            .expect("scope"),
            AuthSurface::Callback,
        );
        let dcr_state = DcrOAuthCallbackState::new(
            state_flow_id,
            scope,
            AuthProviderId::new("notion").expect("provider"),
            CredentialAccountLabel::new("work notion").expect("label"),
            Vec::new(),
        )
        .encode()
        .expect("encoded DCR state");
        let encoded_state =
            url::form_urlencoded::byte_serialize(dcr_state.as_str().as_bytes()).collect::<String>();
        let uri = format!(
            "/api/reborn/product-auth/oauth/callback/{path_flow_id}?state={encoded_state}&code=notion-code"
        )
        .parse::<Uri>()
        .expect("callback uri");

        let error = oauth_callback_handler(
            State(state),
            Path(path_flow_id.to_string()),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            HeaderMap::new(),
        )
        .await
        .expect_err("DCR state bound to another flow must be rejected");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.body.code, AuthErrorCode::MalformedCallback);
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_personal_oauth_start_uses_server_scopes_not_client_supplied_scopes() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let product_auth = Arc::new(RebornProductAuthServices::from_shared(
            shared,
            Arc::new(RecordingDispatcher::default()),
        ));
        let state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_slack_personal_oauth(slack_personal_oauth_test_slot().await);

        let Json(start_response) = extension_oauth_start_handler(
            State(state),
            Extension(WebUiAuthenticatedCaller::new(
                TenantId::new("tenant-alpha").expect("tenant"),
                UserId::new("user-alpha").expect("user"),
                None,
                None,
            )),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack".to_string(),
                scopes: vec!["admin".to_string()],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(InvocationId::new().to_string()),
            }),
        )
        .await
        .expect("start slack oauth flow");

        let parsed =
            Url::parse(start_response.authorization_url.as_str()).expect("authorization url");
        let user_scope = parsed
            .query_pairs()
            .find_map(|(name, value)| (name == "user_scope").then(|| value.into_owned()))
            .expect("Slack user_scope");
        let scopes = user_scope
            .split_whitespace()
            .collect::<std::collections::BTreeSet<_>>();

        assert!(!scopes.contains("admin"));
        for expected in slack_personal_oauth_setup_scopes() {
            assert!(
                scopes.contains(expected),
                "server-authorized Slack scope `{expected}` should be requested"
            );
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_personal_oauth_start_rejects_non_slack_requester_extension() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let product_auth = Arc::new(RebornProductAuthServices::from_shared(
            shared,
            Arc::new(RecordingDispatcher::default()),
        ));
        let state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_slack_personal_oauth(slack_personal_oauth_test_slot().await);

        let error = extension_oauth_start_handler(
            State(state),
            Extension(WebUiAuthenticatedCaller::new(
                TenantId::new("tenant-alpha").expect("tenant"),
                UserId::new("user-alpha").expect("user"),
                None,
                None,
            )),
            Path("notion".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack".to_string(),
                scopes: Vec::new(),
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(InvocationId::new().to_string()),
            }),
        )
        .await
        .expect_err("Slack personal OAuth is only valid for the Slack package");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.body.code, AuthErrorCode::InvalidRequest);
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_personal_oauth_callback_binds_authenticated_user_to_slack_identity() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let provider_identity = OAuthProviderIdentity::new(
            "U123",
            Some("T123".to_string()),
            Some("E123".to_string()),
            Some("A123".to_string()),
        )
        .expect("provider identity");
        let provider_client = Arc::new(SlackIdentityProviderClient::new(provider_identity));
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), dispatcher)
                .with_flow_record_source(shared.clone())
                .with_provider_client(provider_client.clone()),
        );
        let binding_store = Arc::new(RecordingBindingStore::default());
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation");
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            binding_store.clone(),
        ));
        let state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
        .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
            binding_service,
            Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                SlackPersonalConnectionScope {
                    installation_id: installation_id.clone(),
                    team_id: SlackTeamId::new("T123"),
                },
            ))),
            binding_store.clone(),
        ));

        let invocation_id = InvocationId::new();
        let Json(start_response) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(WebUiAuthenticatedCaller::new(
                TenantId::new("tenant-alpha").expect("tenant"),
                UserId::new("user-alpha").expect("user"),
                None,
                None,
            )),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack".to_string(),
                scopes: vec!["search:read".to_string()],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(invocation_id.to_string()),
            }),
        )
        .await
        .expect("start slack oauth flow");
        let state_value = Url::parse(start_response.authorization_url.as_str())
            .expect("authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("oauth state");
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let uri = format!(
            "{SLACK_PERSONAL_OAUTH_CALLBACK_PATH}?state={encoded_state}&code=slack-auth-code"
        )
        .parse::<Uri>()
        .expect("callback uri");

        let response = slack_personal_oauth_callback_handler(
            State(state),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            HeaderMap::new(),
        )
        .await
        .expect("slack callback");

        assert_eq!(response.status(), StatusCode::OK);
        let bindings = binding_store.bindings();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].user_id.as_str(), "user-alpha");
        assert_eq!(bindings[0].provider.as_str(), "slack");
        assert_eq!(bindings[0].provider_user_id.as_str(), "install-alpha:U123");
        assert_eq!(provider_client.calls(), 1);
        assert_eq!(provider_client.cleanup_calls(), 0);
        let owner_scope = AuthProductScope::new(
            ResourceScope {
                tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                user_id: UserId::new("user-alpha").expect("user"),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id,
            },
            AuthSurface::Callback,
        )
        .to_credential_owner();
        let accounts = shared
            .accounts_for_owner(&owner_scope)
            .await
            .expect("list stored account");
        assert_eq!(accounts.len(), 1);
        let stored_identity = accounts[0]
            .provider_identity
            .as_ref()
            .expect("slack oauth should persist non-secret provider identity");
        assert_eq!(stored_identity.subject.as_str(), "U123");
        assert_eq!(stored_identity.team_id.as_deref(), Some("T123"));
        assert_eq!(stored_identity.enterprise_id.as_deref(), Some("E123"));
        assert_eq!(stored_identity.app_id.as_deref(), Some("A123"));
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_personal_oauth_callback_does_not_configure_credential_when_binding_rejects_app()
    {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let provider_identity = OAuthProviderIdentity::new(
            "U123",
            Some("T123".to_string()),
            Some("E123".to_string()),
            Some("A-foreign".to_string()),
        )
        .expect("provider identity");
        let provider_client = Arc::new(SlackIdentityProviderClient::new(provider_identity));
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), dispatcher)
                .with_flow_record_source(shared.clone())
                .with_provider_client(provider_client.clone()),
        );
        let binding_store = Arc::new(RecordingBindingStore::default());
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation");
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            binding_store.clone(),
        ));
        let state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
        .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
            binding_service,
            Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                SlackPersonalConnectionScope {
                    installation_id,
                    team_id: SlackTeamId::new("T123"),
                },
            ))),
            binding_store.clone(),
        ));

        let invocation_id = InvocationId::new();
        let Json(start_response) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(WebUiAuthenticatedCaller::new(
                TenantId::new("tenant-alpha").expect("tenant"),
                UserId::new("user-alpha").expect("user"),
                None,
                None,
            )),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack".to_string(),
                scopes: vec!["users:read".to_string()],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(invocation_id.to_string()),
            }),
        )
        .await
        .expect("start slack oauth flow");
        let state_value = Url::parse(start_response.authorization_url.as_str())
            .expect("authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("oauth state");
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let uri = format!(
            "{SLACK_PERSONAL_OAUTH_CALLBACK_PATH}?state={encoded_state}&code=slack-auth-code"
        )
        .parse::<Uri>()
        .expect("callback uri");

        let error = slack_personal_oauth_callback_handler(
            State(state),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            HeaderMap::new(),
        )
        .await
        .expect_err("foreign Slack app identity must reject callback");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.body.code, AuthErrorCode::MalformedCallback);
        assert_eq!(provider_client.calls(), 1);
        assert_eq!(provider_client.cleanup_calls(), 1);
        assert!(binding_store.bindings().is_empty());
        let owner_scope = AuthProductScope::new(
            ResourceScope {
                tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                user_id: UserId::new("user-alpha").expect("user"),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id,
            },
            AuthSurface::Callback,
        )
        .to_credential_owner();
        let accounts = shared
            .accounts_for_owner(&owner_scope)
            .await
            .expect("list stored accounts");
        assert!(
            accounts.is_empty(),
            "binding rejection must not leave a configured Slack personal credential"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_personal_oauth_callback_rejects_foreign_tenant_state_before_token_exchange() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let provider_identity = OAuthProviderIdentity::new(
            "U123",
            Some("T123".to_string()),
            None,
            Some("A123".to_string()),
        )
        .expect("provider identity");
        let provider_client = Arc::new(SlackIdentityProviderClient::new(provider_identity));
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), dispatcher)
                .with_flow_record_source(shared)
                .with_provider_client(provider_client.clone()),
        );
        let binding_store = Arc::new(RecordingBindingStore::default());
        let installation_id = AdapterInstallationId::new("install-beta").expect("installation");
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: TenantId::new("tenant-beta").expect("tenant"),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            binding_store.clone(),
        ));
        let state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("route tenant"),
            None,
            None,
        )
        .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
        .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
            binding_service,
            Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                SlackPersonalConnectionScope {
                    installation_id,
                    team_id: SlackTeamId::new("T123"),
                },
            ))),
            binding_store.clone(),
        ));

        let invocation_id = InvocationId::new();
        let Json(start_response) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(WebUiAuthenticatedCaller::new(
                TenantId::new("tenant-beta").expect("foreign tenant"),
                UserId::new("user-beta").expect("foreign user"),
                None,
                None,
            )),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack".to_string(),
                scopes: vec!["search:read".to_string()],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(invocation_id.to_string()),
            }),
        )
        .await
        .expect("start foreign-tenant slack oauth flow");
        let state_value = Url::parse(start_response.authorization_url.as_str())
            .expect("authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("oauth state");
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let uri = format!(
            "{SLACK_PERSONAL_OAUTH_CALLBACK_PATH}?state={encoded_state}&code=slack-auth-code"
        )
        .parse::<Uri>()
        .expect("callback uri");

        let error = slack_personal_oauth_callback_handler(
            State(state),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            HeaderMap::new(),
        )
        .await
        .expect_err("foreign tenant callback state must be rejected");

        assert_eq!(error.status, StatusCode::FORBIDDEN);
        assert_eq!(error.body.code, AuthErrorCode::CrossScopeDenied);
        assert_eq!(provider_client.calls(), 0);
        assert!(binding_store.bindings().is_empty());
    }

    #[derive(Default)]
    struct RecordingDispatcher {
        events: Mutex<Vec<ironclaw_auth::AuthContinuationEvent>>,
    }

    impl RecordingDispatcher {
        fn events(&self) -> Vec<ironclaw_auth::AuthContinuationEvent> {
            self.events
                .lock()
                .expect("recording dispatcher lock")
                .clone()
        }
    }

    #[async_trait]
    impl RebornAuthContinuationDispatcher for RecordingDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            event: ironclaw_auth::AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            self.events
                .lock()
                .expect("recording dispatcher lock")
                .push(event);
            Ok(())
        }
    }

    #[derive(Clone)]
    struct SlackIdentityProviderClient {
        provider_identity: OAuthProviderIdentity,
        calls: Arc<AtomicUsize>,
        cleanup_calls: Arc<AtomicUsize>,
    }

    impl SlackIdentityProviderClient {
        fn new(provider_identity: OAuthProviderIdentity) -> Self {
            Self {
                provider_identity,
                calls: Arc::new(AtomicUsize::new(0)),
                cleanup_calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn cleanup_calls(&self) -> usize {
            self.cleanup_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl AuthProviderClient for SlackIdentityProviderClient {
        async fn exchange_callback(
            &self,
            _context: OAuthProviderExchangeContext,
            request: OAuthProviderCallbackRequest,
        ) -> Result<OAuthProviderExchange, AuthProductError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(OAuthProviderExchange {
                provider: request.provider,
                account_label: request.account_label,
                authorization_code_hash: request.authorization_code_hash,
                pkce_verifier_hash: request.pkce_verifier_hash,
                access_secret: SecretHandle::new("slack-access").expect("access handle"),
                refresh_secret: Some(SecretHandle::new("slack-refresh").expect("refresh handle")),
                scopes: request.scopes,
                account_id: None,
                provider_identity: Some(self.provider_identity.clone()),
            })
        }

        async fn refresh_token(
            &self,
            request: OAuthProviderRefreshRequest,
        ) -> Result<OAuthProviderRefresh, AuthProductError> {
            Ok(OAuthProviderRefresh {
                provider: request.provider,
                access_secret: SecretHandle::new("slack-refreshed-access")
                    .expect("refreshed access handle"),
                refresh_secret: Some(
                    SecretHandle::new("slack-refreshed-refresh").expect("refreshed refresh handle"),
                ),
                scopes: request.scopes,
            })
        }

        async fn cleanup_exchange(
            &self,
            _context: OAuthProviderExchangeContext,
            _exchange: &OAuthProviderExchange,
        ) -> Result<(), AuthProductError> {
            self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[derive(Default)]
    struct RecordingBindingStore {
        bindings: Mutex<Vec<RebornUserIdentityBinding>>,
    }

    #[cfg(feature = "slack-v2-host-beta")]
    impl RecordingBindingStore {
        fn bindings(&self) -> Vec<RebornUserIdentityBinding> {
            self.bindings.lock().expect("binding store lock").clone()
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[async_trait]
    impl RebornUserIdentityBindingStore for RecordingBindingStore {
        async fn bind_user_identity(
            &self,
            binding: RebornUserIdentityBinding,
        ) -> Result<(), RebornUserIdentityBindingError> {
            self.bindings
                .lock()
                .expect("binding store lock")
                .push(binding);
            Ok(())
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[async_trait]
    impl crate::slack::slack_personal_binding::RebornUserIdentityBindingDeleteStore
        for RecordingBindingStore
    {
        async fn delete_user_identity_bindings_for_user(
            &self,
            provider: &str,
            user_id: &ironclaw_host_api::UserId,
            provider_user_id_prefix: Option<&str>,
        ) -> Result<usize, RebornUserIdentityBindingError> {
            let mut bindings = self.bindings.lock().expect("binding store lock");
            let before = bindings.len();
            bindings.retain(|binding| {
                !(binding.provider.as_str() == provider
                    && binding.user_id == *user_id
                    && provider_user_id_prefix
                        .is_none_or(|prefix| binding.provider_user_id.as_str().starts_with(prefix)))
            });
            Ok(before - bindings.len())
        }
    }

    /// Delegates every flow operation to the in-memory fake but fails
    /// `complete_oauth_callback`, modeling a completion failure (flow-store
    /// IO, CAS mismatch) that lands after the identity hook already bound.
    #[cfg(feature = "slack-v2-host-beta")]
    struct FailingCompletionFlowManager {
        inner: Arc<InMemoryAuthProductServices>,
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[async_trait]
    impl ironclaw_auth::AuthFlowManager for FailingCompletionFlowManager {
        async fn create_flow(
            &self,
            request: ironclaw_auth::NewAuthFlow,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            self.inner.create_flow(request).await
        }

        async fn get_flow(
            &self,
            scope: &AuthProductScope,
            flow_id: ironclaw_auth::AuthFlowId,
        ) -> Result<Option<ironclaw_auth::AuthFlowRecord>, AuthProductError> {
            self.inner.get_flow(scope, flow_id).await
        }

        async fn claim_oauth_callback(
            &self,
            scope: &AuthProductScope,
            request: ironclaw_auth::OAuthCallbackClaimRequest,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            self.inner.claim_oauth_callback(scope, request).await
        }

        async fn complete_oauth_callback(
            &self,
            _scope: &AuthProductScope,
            _input: ironclaw_auth::OAuthCallbackInput,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            Err(AuthProductError::BackendUnavailable)
        }

        async fn complete_credential_selection(
            &self,
            scope: &AuthProductScope,
            input: ironclaw_auth::CredentialSelectionInput,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            self.inner.complete_credential_selection(scope, input).await
        }

        async fn complete_manual_token(
            &self,
            scope: &AuthProductScope,
            input: ironclaw_auth::ManualTokenCompletionInput,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            self.inner.complete_manual_token(scope, input).await
        }

        async fn cancel_manual_token(
            &self,
            scope: &AuthProductScope,
            interaction_id: ironclaw_auth::AuthInteractionId,
        ) -> Result<Option<ironclaw_auth::AuthFlowRecord>, AuthProductError> {
            self.inner.cancel_manual_token(scope, interaction_id).await
        }

        async fn fail_oauth_callback(
            &self,
            scope: &AuthProductScope,
            input: ironclaw_auth::OAuthCallbackFailureInput,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            self.inner.fail_oauth_callback(scope, input).await
        }

        async fn mark_continuation_dispatched(
            &self,
            scope: &AuthProductScope,
            flow_id: ironclaw_auth::AuthFlowId,
            emitted_at: ironclaw_auth::Timestamp,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            self.inner
                .mark_continuation_dispatched(scope, flow_id, emitted_at)
                .await
        }

        async fn cancel_flow(
            &self,
            scope: &AuthProductScope,
            flow_id: ironclaw_auth::AuthFlowId,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            self.inner.cancel_flow(scope, flow_id).await
        }
    }

    /// Completion failure after the identity hook bound must roll the binding
    /// back: the binding is the user-visible "connected" signal, and the
    /// completed-flow replay path never re-runs the hook, so a surviving
    /// binding would show Slack connected with no usable credential.
    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_personal_oauth_callback_rolls_back_binding_when_completion_fails() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let provider_identity = OAuthProviderIdentity::new(
            "U123",
            Some("T123".to_string()),
            Some("E123".to_string()),
            Some("A123".to_string()),
        )
        .expect("provider identity");
        let provider_client = Arc::new(SlackIdentityProviderClient::new(provider_identity));
        let failing_flows = Arc::new(FailingCompletionFlowManager {
            inner: shared.clone(),
        });
        let product_auth = Arc::new(RebornProductAuthServices::new(
            failing_flows,
            shared.clone(),
            shared.clone(),
            shared.clone(),
            provider_client.clone(),
            shared.clone(),
            dispatcher,
        ));
        let binding_store = Arc::new(RecordingBindingStore::default());
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation");
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            binding_store.clone(),
        ));
        let state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
        .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
            binding_service,
            Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                SlackPersonalConnectionScope {
                    installation_id,
                    team_id: SlackTeamId::new("T123"),
                },
            ))),
            binding_store.clone(),
        ));

        let invocation_id = InvocationId::new();
        let Json(start_response) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(WebUiAuthenticatedCaller::new(
                TenantId::new("tenant-alpha").expect("tenant"),
                UserId::new("user-alpha").expect("user"),
                None,
                None,
            )),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack".to_string(),
                scopes: vec!["search:read".to_string()],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(invocation_id.to_string()),
            }),
        )
        .await
        .expect("start slack oauth flow");
        let state_value = Url::parse(start_response.authorization_url.as_str())
            .expect("authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("oauth state");
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let uri = format!(
            "{SLACK_PERSONAL_OAUTH_CALLBACK_PATH}?state={encoded_state}&code=slack-auth-code"
        )
        .parse::<Uri>()
        .expect("callback uri");

        slack_personal_oauth_callback_handler(
            State(state),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            HeaderMap::new(),
        )
        .await
        .expect_err("completion failure must surface as a callback error");

        assert_eq!(provider_client.calls(), 1, "token exchange ran");
        assert_eq!(
            provider_client.cleanup_calls(),
            1,
            "token material must be cleaned up on completion failure"
        );
        assert!(
            binding_store.bindings().is_empty(),
            "identity binding written by the hook must be rolled back when completion fails"
        );
    }
}
