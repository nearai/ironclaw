//! OAuth start and callback handlers.

// arch-exempt: large_file, shared provider-neutral OAuth callback engine and tests, plan #5905

use std::{future::Future, pin::Pin};

use super::*;
use crate::product_auth::api::auth::OAuthProviderIdentityCheck;
use crate::product_auth::oauth::oauth_dcr::DcrOAuthCallbackState;

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
                continuation: AuthContinuationRef::SetupOnly,
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
    let (scope, flow_id) = oauth_flow_scope(&caller, &flow_id, query)?;
    let flow = run_with_backend_timeout(state.product_auth.flow_record_for_status(&scope, flow_id))
        .await?;
    Ok(Json(OAuthFlowStatusResponse {
        status: flow.status,
    }))
}

/// Explicit recovery command for a durable OAuth flow.
///
/// Unlike [`oauth_flow_status_handler`], this route may claim and dispatch a
/// pending lifecycle continuation or converge its exact compensation and
/// provider-owned cleanup journals. The entire command, including terminal
/// provider hooks, runs under one backend deadline.
pub(super) async fn oauth_flow_reconcile_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(flow_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<OAuthFlowStatusQuery>,
) -> Result<Json<OAuthFlowStatusResponse>, ProductAuthRouteFailure> {
    let (scope, flow_id) = oauth_flow_scope(&caller, &flow_id, query)?;
    let status = run_with_backend_timeout(reconcile_oauth_flow(&state, &scope, flow_id)).await?;
    Ok(Json(OAuthFlowStatusResponse { status }))
}

fn oauth_flow_scope(
    caller: &WebUiAuthenticatedCaller,
    flow_id: &str,
    query: OAuthFlowStatusQuery,
) -> Result<(AuthProductScope, AuthFlowId), ProductAuthRouteFailure> {
    let flow_id = AuthFlowId::from_uuid(Uuid::parse_str(flow_id).map_err(|error| {
        tracing::debug!(%error, "malformed flow id in oauth flow status/reconcile path");
        ProductAuthRouteFailure::malformed_callback()
    })?);
    let fields = ScopeFields {
        session_id: None,
        thread_id: None,
        invocation_id: query.invocation_id,
    };
    let scope = scope_from_authenticated_caller_parts_requiring_invocation(caller, &fields)?;
    Ok((scope, flow_id))
}

async fn reconcile_oauth_flow(
    state: &ProductAuthRouteState,
    scope: &AuthProductScope,
    flow_id: AuthFlowId,
) -> Result<AuthFlowStatus, ProductAuthRouteFailure> {
    let before = state
        .product_auth
        .flow_record_for_status(scope, flow_id)
        .await
        .map_err(ProductAuthRouteFailure::from)?;
    let terminal_cleanup_error = if before.status == AuthFlowStatus::Failed {
        if let Some(descriptor) = oauth_callback_descriptor_for_provider(&before.provider) {
            terminal_failure_hook(
                state,
                descriptor,
                scope,
                flow_id,
                RebornOAuthCallbackFailureStage::ContinuationSideEffect,
            )
            .await
            .err()
        } else {
            None
        }
    } else {
        None
    };
    // Credential compensation and provider-owned binding cleanup are
    // independent journals. Always attempt both; report failure until both
    // converge so the browser keeps polling.
    let status = state
        .product_auth
        .reconcile_oauth_flow(scope, flow_id)
        .await
        .map_err(ProductAuthRouteFailure::from)?;
    if status == AuthFlowStatus::Failed && before.status != AuthFlowStatus::Failed {
        let after = state
            .product_auth
            .flow_record_for_status(scope, flow_id)
            .await
            .map_err(ProductAuthRouteFailure::from)?;
        let provider = after.provider;
        if let Some(descriptor) = oauth_callback_descriptor_for_provider(&provider) {
            terminal_failure_hook(
                state,
                descriptor,
                scope,
                flow_id,
                RebornOAuthCallbackFailureStage::ContinuationSideEffect,
            )
            .await?;
        }
    }
    if let Some(error) = terminal_cleanup_error {
        return Err(error);
    }
    Ok(status)
}

pub(super) async fn abort_started_extension_oauth_flow(
    state: &ProductAuthRouteState,
    response: &ProductOAuthStartResponse,
) -> Result<(), ProductAuthRouteFailure> {
    let mut scope = AuthProductScope::new(
        ResourceScope {
            tenant_id: state.tenant_id.clone(),
            user_id: response.callback_scope.user_id.clone(),
            agent_id: response.callback_scope.agent_id.clone(),
            project_id: response.callback_scope.project_id.clone(),
            mission_id: None,
            thread_id: response.callback_scope.thread_id.clone(),
            invocation_id: response.callback_scope.invocation_id,
        },
        AuthSurface::Callback,
    );
    if let Some(session_id) = response.callback_scope.session_id.clone() {
        scope = scope.with_session_id(session_id);
    }
    if let Some(descriptor) = oauth_callback_descriptor_for_provider(&response.provider) {
        run_terminal_failure_hook(
            state,
            descriptor,
            &scope,
            response.flow_id,
            RebornOAuthCallbackFailureStage::Terminal,
        )
        .await?;
    }
    run_with_backend_timeout(
        state
            .product_auth
            .flow_manager()
            .cancel_flow(&scope, response.flow_id),
    )
    .await?;
    state.remove_pkce_verifier(response.flow_id);
    Ok(())
}

pub(super) async fn google_oauth_start_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<GoogleOAuthStartRequest>,
) -> Result<Json<ProductOAuthStartResponse>, ProductAuthRouteFailure> {
    start_google_oauth_flow(state, caller, request, None, false).await
}

pub(crate) async fn start_extension_oauth_flow(
    state: ProductAuthRouteState,
    caller: WebUiAuthenticatedCaller,
    request: ExtensionOAuthStartRequest,
    requester_extension: ExtensionId,
) -> Result<Json<ProductOAuthStartResponse>, ProductAuthRouteFailure> {
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

fn extension_lifecycle_continuation(
    requester_extension: &ExtensionId,
) -> Result<AuthContinuationRef, ProductAuthRouteFailure> {
    let package_ref = ironclaw_auth::LifecyclePackageRef::new(requester_extension.as_str())
        .map_err(|error| {
            tracing::error!(%error, extension_id = %requester_extension, "validated extension id could not form an auth lifecycle package ref");
            ProductAuthRouteFailure::backend_unavailable()
        })?;
    Ok(AuthContinuationRef::LifecycleActivation { package_ref })
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
            continuation: extension_lifecycle_continuation(&requester_extension)?,
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

    let flow = run_with_backend_timeout(
        state
            .product_auth
            .start_setup_oauth_flow(RebornOAuthStartFlowRequest {
                flow_id: Some(flow_id),
                scope: scope.clone(),
                provider: provider.clone(),
                authorization_url: authorization_url.clone(),
                opaque_state_hash: opaque_state_hash.clone(),
                pkce_verifier_hash,
                continuation: requester_extension
                    .as_ref()
                    .map(extension_lifecycle_continuation)
                    .transpose()?
                    .unwrap_or(AuthContinuationRef::SetupOnly),
                update_binding,
                expires_at: request.expires_at,
            }),
    )
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
pub(crate) enum CallbackScopeResolution {
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
pub(crate) type OAuthCallbackTerminalHookFuture =
    Pin<Box<dyn Future<Output = Result<(), ProductAuthRouteFailure>> + Send>>;
pub(crate) type OAuthCallbackTerminalHook = fn(
    ProductAuthRouteState,
    AuthProductScope,
    AuthFlowId,
    RebornOAuthCallbackFailureStage,
) -> OAuthCallbackTerminalHookFuture;

pub(crate) struct OAuthCallbackDescriptor {
    /// Wire prefix + scope policy used to decode the callback `state`.
    pub(crate) state_kind: OAuthCallbackStateKind,
    /// Reborn provider id submitted with the token exchange.
    pub(crate) provider_id: &'static str,
    /// How granted scopes are resolved at the callback.
    pub(crate) scope_resolution: CallbackScopeResolution,
    /// Optional post-exchange provider-identity check (Slack binds the
    /// `authed_user` identity; Google has none). Built after the callback scope
    /// is decoded so it can capture the scope.
    pub(crate) identity_hook: fn(
        &ProductAuthRouteState,
        &AuthProductScope,
        AuthFlowId,
    ) -> Option<OAuthProviderIdentityCheck>,
    /// Provider-owned cleanup after a terminal callback outcome. The shared
    /// engine decides when an outcome is terminal; providers own any external
    /// lifecycle state that must converge with the durable auth flow.
    pub(crate) on_terminal_failure: Option<OAuthCallbackTerminalHook>,
}

/// No post-exchange identity check (Google).
fn no_identity_hook(
    _state: &ProductAuthRouteState,
    _callback_scope: &AuthProductScope,
    _flow_id: AuthFlowId,
) -> Option<OAuthProviderIdentityCheck> {
    None
}

static GOOGLE_CALLBACK_DESCRIPTOR: OAuthCallbackDescriptor = OAuthCallbackDescriptor {
    state_kind: OAuthCallbackStateKind::GOOGLE,
    provider_id: GOOGLE_PROVIDER_ID,
    scope_resolution: CallbackScopeResolution::ValidateEchoedIncludesRequested,
    identity_hook: no_identity_hook,
    on_terminal_failure: None,
};

fn oauth_callback_descriptor_for_provider(
    provider: &AuthProviderId,
) -> Option<&'static OAuthCallbackDescriptor> {
    if provider.as_str() == GOOGLE_PROVIDER_ID {
        return Some(&GOOGLE_CALLBACK_DESCRIPTOR);
    }
    #[cfg(feature = "slack-v2-host-beta")]
    if provider.as_str() == SLACK_PERSONAL_PROVIDER_ID {
        return Some(&crate::slack::slack_personal_oauth::SLACK_PERSONAL_CALLBACK_DESCRIPTOR);
    }
    None
}

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
pub(crate) async fn oauth_provider_callback_handler(
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
        run_terminal_failure_hook_best_effort(
            &state,
            descriptor,
            callback_scope,
            flow_id,
            RebornOAuthCallbackFailureStage::Terminal,
        )
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
    // From this point the callback is tied to a known, scoped flow. Any error
    // means this provider redirect cannot make further progress (the code or
    // one-shot PKCE material is absent/consumed, or product-auth terminalized
    // the flow). Provider cleanup therefore follows the control-flow outcome,
    // not a hand-maintained list of error codes.
    let mut callback_owned_by_service = false;
    let mut terminal_failure_hook_attempted = false;
    let result = async {
        let code = query
            .code
            .as_ref()
            .ok_or_else(ProductAuthRouteFailure::malformed_callback)?;
        let pkce_verifier =
            pkce_verifier_for_known_callback_flow(&state, callback_scope, &provider, flow_id)
                .await?;
        let callback_scopes = match resolve_callback_scopes(
            descriptor.scope_resolution,
            callback_state.requested_scopes(),
            query.scopes.as_deref(),
        )? {
            CallbackScopeOutcome::Scopes(scopes) => scopes,
            CallbackScopeOutcome::ProviderDenied => {
                state.remove_pkce_verifier(flow_id);
                callback_owned_by_service = true;
                let response = run_with_backend_timeout(state.product_auth.handle_oauth_callback(
                    RebornOAuthCallbackRequest {
                        scope: callback_scope.clone(),
                        flow_id,
                        opaque_state_hash: state_hash.clone(),
                        outcome: RebornOAuthCallbackOutcome::ProviderDenied,
                    },
                ))
                .await;
                terminal_failure_hook_attempted = true;
                run_terminal_failure_hook_best_effort(
                    &state,
                    descriptor,
                    callback_scope,
                    flow_id,
                    RebornOAuthCallbackFailureStage::Terminal,
                )
                .await;
                return oauth_callback_route_result_response(headers, response);
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
        let identity_check = (descriptor.identity_hook)(&state, callback_scope, flow_id);
        callback_owned_by_service = true;
        let response = run_with_backend_timeout(
            state
                .product_auth
                .handle_oauth_callback_with_optional_provider_identity_check(
                    callback_request,
                    identity_check,
                ),
        )
        .await?;
        state.remove_pkce_verifier(flow_id);
        Ok(oauth_callback_response(headers, response))
    }
    .await;
    if let Err(error) = &result {
        let stage = error.callback_failure_stage;
        if !callback_owned_by_service {
            terminalize_known_malformed_callback(&state, callback_scope, flow_id, state_hash)
                .await?;
        }
        if !matches!(
            stage,
            RebornOAuthCallbackFailureStage::ContinuationAcknowledgement
                | RebornOAuthCallbackFailureStage::ContinuationRetryable
                | RebornOAuthCallbackFailureStage::ContinuationCompensation
        ) {
            state.remove_pkce_verifier(flow_id);
        }
        if !terminal_failure_hook_attempted
            && !matches!(
                stage,
                RebornOAuthCallbackFailureStage::ContinuationAcknowledgement
                    | RebornOAuthCallbackFailureStage::ContinuationRetryable
            )
        {
            run_terminal_failure_hook_best_effort(
                &state,
                descriptor,
                callback_scope,
                flow_id,
                stage,
            )
            .await;
        }
    }
    result
}

async fn terminalize_known_malformed_callback(
    state: &ProductAuthRouteState,
    callback_scope: &AuthProductScope,
    flow_id: AuthFlowId,
    state_hash: OpaqueStateHash,
) -> Result<(), ProductAuthRouteFailure> {
    match run_with_backend_timeout(state.product_auth.handle_oauth_callback(
        RebornOAuthCallbackRequest {
            scope: callback_scope.clone(),
            flow_id,
            opaque_state_hash: state_hash,
            outcome: RebornOAuthCallbackOutcome::Malformed,
        },
    ))
    .await
    {
        Err(error) if error.body.code == AuthErrorCode::MalformedCallback => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Err(ProductAuthRouteFailure::backend_unavailable()),
    }
}

async fn run_terminal_failure_hook(
    state: &ProductAuthRouteState,
    descriptor: &OAuthCallbackDescriptor,
    callback_scope: &AuthProductScope,
    flow_id: AuthFlowId,
    failure_stage: RebornOAuthCallbackFailureStage,
) -> Result<(), ProductAuthRouteFailure> {
    run_with_backend_timeout(terminal_failure_hook(
        state,
        descriptor,
        callback_scope,
        flow_id,
        failure_stage,
    ))
    .await
}

async fn run_terminal_failure_hook_best_effort(
    state: &ProductAuthRouteState,
    descriptor: &OAuthCallbackDescriptor,
    callback_scope: &AuthProductScope,
    flow_id: AuthFlowId,
    failure_stage: RebornOAuthCallbackFailureStage,
) {
    if let Err(hook_error) =
        run_terminal_failure_hook(state, descriptor, callback_scope, flow_id, failure_stage).await
    {
        tracing::warn!(
            error_code = ?hook_error.body.code,
            %flow_id,
            "provider terminal cleanup remains pending for status polling"
        );
    }
}

async fn terminal_failure_hook(
    state: &ProductAuthRouteState,
    descriptor: &OAuthCallbackDescriptor,
    callback_scope: &AuthProductScope,
    flow_id: AuthFlowId,
    failure_stage: RebornOAuthCallbackFailureStage,
) -> Result<(), ProductAuthRouteFailure> {
    if let Some(hook) = descriptor.on_terminal_failure {
        return hook(
            state.clone(),
            callback_scope.clone(),
            flow_id,
            failure_stage,
        )
        .await;
    }
    Ok(())
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
        AuthErrorCode::ProviderIdentityAlreadyConnected => {
            "This provider account is already connected to another Reborn user. Disconnect it from that user, then try again."
        }
        AuthErrorCode::ConnectionConflict => {
            "This connection is already active or changing. Wait for the current operation to finish, or disconnect it before trying again."
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
            | AuthErrorCode::ProviderIdentityAlreadyConnected
            | AuthErrorCode::ConnectionConflict
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
    #[cfg(feature = "slack-v2-host-beta")]
    use ironclaw_auth::{
        AuthFlowManager, AuthProviderClient, CredentialAccountLabel, CredentialAccountRecordSource,
        CredentialAccountService, CredentialOwnership, NewCredentialAccount, OAuthProviderExchange,
        OAuthProviderExchangeContext, OAuthProviderIdentity, OAuthProviderRefresh,
        OAuthProviderRefreshRequest,
    };
    use ironclaw_auth::{GOOGLE_CALENDAR_READONLY_SCOPE, InMemoryAuthProductServices};
    #[cfg(feature = "slack-v2-host-beta")]
    use ironclaw_host_api::SecretHandle;
    use ironclaw_host_api::{RuntimeCredentialAccountProviderId, RuntimeCredentialAuthRequirement};
    #[cfg(feature = "slack-v2-host-beta")]
    use ironclaw_product_adapters::AdapterInstallationId;
    use ironclaw_secrets::{InMemorySecretStore, SecretStore};
    use ironclaw_turns::{TurnRunId, TurnScope};
    #[cfg(feature = "slack-v2-host-beta")]
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[cfg(feature = "slack-v2-host-beta")]
    use crate::extension_host::available_extensions::slack_personal_oauth_setup_scopes;
    #[cfg(feature = "slack-v2-host-beta")]
    use crate::slack::slack_host_beta::{
        SlackPersonalConnectionScope, SlackPersonalConnectionScopeResolver,
        StaticSlackPersonalConnectionScopeResolver,
    };
    #[cfg(feature = "slack-v2-host-beta")]
    use crate::slack::slack_personal_binding::{
        RebornIdentityProviderId, RebornIdentityProviderUserId, RebornUserIdentityBinding,
        RebornUserIdentityBindingError, RebornUserIdentityBindingStore,
        SlackConnectionCleanupSelector, SlackConnectionEpoch, SlackConnectionOwner,
        SlackConnectionState, SlackDisconnectFence, SlackPersonalBindingInstallation,
        SlackPersonalUserBindingService, SlackUserBindingLifecycleError,
        SlackUserBindingLifecycleStore, SlackUserIdentityCleanupBinding,
    };
    #[cfg(feature = "slack-v2-host-beta")]
    use crate::slack::slack_personal_oauth::{
        SLACK_PERSONAL_CALLBACK_DESCRIPTOR, SlackPersonalOAuthGateLifecycle,
        SlackPersonalOAuthGateProvider, slack_lifecycle_start_failure,
        slack_personal_oauth_callback_handler,
    };
    #[cfg(feature = "slack-v2-host-beta")]
    use crate::slack::slack_serve::SlackInstallationSelector;
    #[cfg(feature = "slack-v2-host-beta")]
    use crate::slack::slack_setup::{
        SlackInstallationSetup, SlackInstallationSetupStore, SlackInstallationSetupUpdate,
        SlackPersonalSetupServiceSlot, SlackSetupError, SlackSetupService,
    };

    static FAILING_TERMINAL_HOOK_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn failing_terminal_hook(
        _state: ProductAuthRouteState,
        _scope: AuthProductScope,
        _flow_id: AuthFlowId,
        _stage: RebornOAuthCallbackFailureStage,
    ) -> OAuthCallbackTerminalHookFuture {
        Box::pin(async {
            FAILING_TERMINAL_HOOK_CALLS.fetch_add(1, Ordering::SeqCst);
            Err(ProductAuthRouteFailure::backend_unavailable())
        })
    }

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

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_personal_turn_gate_callback_activates_binding_lifecycle_and_account() {
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
        let tenant_id = TenantId::new("tenant-alpha").expect("tenant");
        let user_id = UserId::new("user-alpha").expect("user");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation");
        let owner =
            SlackConnectionOwner::new(tenant_id.clone(), user_id.clone(), installation_id.clone());
        let lifecycle_store = Arc::new(TestSlackLifecycleStore::default());
        let lifecycle_port: Arc<dyn SlackUserBindingLifecycleStore> = lifecycle_store.clone();
        let connection_scope_resolver: Arc<dyn SlackPersonalConnectionScopeResolver> = Arc::new(
            StaticSlackPersonalConnectionScopeResolver::new(Some(SlackPersonalConnectionScope {
                installation_id: installation_id.clone(),
            })),
        );
        let binding_store = Arc::new(RecordingBindingStore::default());
        let activating_binding_store: Arc<dyn RebornUserIdentityBindingStore> =
            Arc::new(ActivatingBindingStore {
                inner: binding_store.clone(),
                lifecycle_store: lifecycle_store.clone(),
                owner: owner.clone(),
            });
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant_id.clone(),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            activating_binding_store,
        ));
        let slot = slack_personal_oauth_test_slot().await;
        slot.fill_gate_lifecycle(SlackPersonalOAuthGateLifecycle::new(
            connection_scope_resolver.clone(),
            lifecycle_port.clone(),
        ));
        let gate_driver = Arc::new(OAuthGateFlowDriver::new(
            Arc::new(SlackPersonalOAuthGateProvider::new(slot.clone())),
            Arc::new(InMemorySecretStore::new()),
        ));
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), dispatcher.clone())
                .with_flow_record_source(shared.clone())
                .with_provider_client(provider_client)
                .with_oauth_gate_registry(Arc::new(OAuthGateProviderRegistry::new(vec![
                    gate_driver,
                ]))),
        );
        let state = ProductAuthRouteState::new(product_auth.clone(), tenant_id.clone(), None, None)
            .with_test_installed_extension_lookup()
            .with_slack_personal_oauth(slot)
            .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
                binding_service,
                connection_scope_resolver,
                binding_store.clone(),
                lifecycle_port,
            ));
        let turn_scope = TurnScope::new(
            tenant_id.clone(),
            None,
            None,
            ThreadId::new("thread-alpha").expect("thread"),
        );
        let run_id = TurnRunId::new();
        let gate_ref = "gate:slack-personal-auth";
        let requirements = vec![RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new(SLACK_PERSONAL_PROVIDER_ID)
                .expect("provider"),
            setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
                scopes: slack_personal_oauth_setup_scopes()
                    .iter()
                    .map(|scope| (*scope).to_string())
                    .collect(),
            },
            requester_extension: ExtensionId::new("slack").expect("extension"),
            provider_scopes: slack_personal_oauth_setup_scopes()
                .iter()
                .map(|scope| (*scope).to_string())
                .collect(),
        }];

        let challenge = product_auth
            .challenge_for_gate(&turn_scope, &user_id, run_id, gate_ref, &requirements)
            .await
            .expect("challenge lookup")
            .expect("Slack OAuth challenge");
        let authorization_url = challenge.authorization_url.expect("authorization url");
        let second_turn_scope = TurnScope::new(
            tenant_id.clone(),
            None,
            None,
            ThreadId::new("thread-beta").expect("second thread"),
        );
        let second_challenge = product_auth
            .challenge_for_gate(
                &second_turn_scope,
                &user_id,
                TurnRunId::new(),
                gate_ref,
                &requirements,
            )
            .await
            .expect("second challenge lookup")
            .expect("second Slack OAuth challenge");
        assert_eq!(
            second_challenge.authorization_url.as_ref(),
            Some(&authorization_url),
            "another blocked thread must reuse the in-progress Slack OAuth challenge"
        );
        assert_eq!(
            shared
                .flow_records_snapshot()
                .into_iter()
                .filter(|flow| flow.status == AuthFlowStatus::AwaitingUser)
                .count(),
            1,
            "one caller-wide Slack connection attempt must back every blocked thread"
        );
        let first_state_value = Url::parse(authorization_url.as_str())
            .expect("first authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("first oauth state");
        let first_callback_state = OAuthCallbackState::decode(
            OAuthCallbackStateKind::SLACK_PERSONAL,
            first_state_value.as_str(),
        )
        .expect("first callback state");
        shared
            .cancel_flow(first_callback_state.scope(), first_callback_state.flow_id())
            .await
            .expect("cancel first shared flow");

        let third_turn_scope = TurnScope::new(
            tenant_id.clone(),
            None,
            None,
            ThreadId::new("thread-gamma").expect("third thread"),
        );
        let third_run_id = TurnRunId::new();
        let third_challenge = product_auth
            .challenge_for_gate(
                &third_turn_scope,
                &user_id,
                third_run_id,
                gate_ref,
                &requirements,
            )
            .await
            .expect("replacement challenge lookup")
            .expect("replacement Slack OAuth challenge");
        let replacement_authorization_url = third_challenge
            .authorization_url
            .expect("replacement authorization url");
        assert_ne!(
            replacement_authorization_url, authorization_url,
            "a canceled shared Slack flow must be replaced immediately"
        );
        let authorization_url = replacement_authorization_url;
        let stale_state_value = Url::parse(authorization_url.as_str())
            .expect("authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("oauth state");
        let stale_callback_state = OAuthCallbackState::decode(
            OAuthCallbackStateKind::SLACK_PERSONAL,
            stale_state_value.as_str(),
        )
        .expect("callback state");
        let epoch = SlackConnectionEpoch::new(stale_callback_state.flow_id());
        assert_eq!(
            lifecycle_store
                .connection_state(&owner)
                .await
                .expect("connection state"),
            Some((epoch, SlackConnectionState::Connecting)),
            "production gate setup must fence the Slack connection before rendering OAuth"
        );
        lifecycle_store.transition_after_next_state_read(SlackConnectionState::Disconnecting);
        let stale_reuse = product_auth
            .challenge_for_gate(
                &TurnScope::new(
                    tenant_id.clone(),
                    None,
                    None,
                    ThreadId::new("thread-delta").expect("fourth thread"),
                ),
                &user_id,
                TurnRunId::new(),
                gate_ref,
                &requirements,
            )
            .await;
        assert!(
            matches!(stale_reuse, Err(AuthProductError::BackendConflict)),
            "a flow must not be reused after its lifecycle epoch changes concurrently"
        );
        lifecycle_store.set_state(&owner, epoch, SlackConnectionState::Connecting);
        lifecycle_store.transition_after_next_state_read(SlackConnectionState::Disconnecting);
        let stale_exact_reuse = product_auth
            .challenge_for_gate(
                &third_turn_scope,
                &user_id,
                third_run_id,
                gate_ref,
                &requirements,
            )
            .await;
        assert!(
            matches!(stale_exact_reuse, Err(AuthProductError::BackendConflict)),
            "an exact turn-gate flow must still be revalidated against Slack lifecycle state"
        );
        lifecycle_store.set_state(&owner, epoch, SlackConnectionState::Connecting);
        let callback_challenge = product_auth
            .challenge_for_gate(
                &third_turn_scope,
                &user_id,
                third_run_id,
                gate_ref,
                &requirements,
            )
            .await
            .expect("fresh challenge lookup after stale flow retirement")
            .expect("fresh Slack OAuth challenge");
        let callback_authorization_url = callback_challenge
            .authorization_url
            .expect("fresh authorization url");
        assert_ne!(
            callback_authorization_url, authorization_url,
            "a lifecycle-rejected flow must be retired before retry"
        );
        let state_value = Url::parse(callback_authorization_url.as_str())
            .expect("fresh authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("fresh oauth state");
        let callback_state = OAuthCallbackState::decode(
            OAuthCallbackStateKind::SLACK_PERSONAL,
            state_value.as_str(),
        )
        .expect("fresh callback state");
        let epoch = SlackConnectionEpoch::new(callback_state.flow_id());
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let uri = format!(
            "{SLACK_PERSONAL_OAUTH_CALLBACK_PATH}?state={encoded_state}&code=slack-auth-code"
        )
        .parse::<Uri>()
        .expect("callback uri");

        let response = slack_personal_oauth_callback_handler(
            State(state.clone()),
            RawQuery(uri.query().map(str::to_string)), // safety: URI query parsing, not a database query.
            uri,
            HeaderMap::new(),
        )
        .await
        .expect("Slack callback");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            lifecycle_store
                .connection_state(&owner)
                .await
                .expect("connection state"),
            Some((epoch, SlackConnectionState::Active))
        );
        assert_eq!(binding_store.bindings().len(), 1);
        let accounts = shared
            .accounts_for_owner(&callback_state.scope().to_credential_owner())
            .await
            .expect("configured account");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].status, CredentialAccountStatus::Configured);
        assert_eq!(dispatcher.events().len(), 1);
        assert_eq!(
            dispatcher.events()[0].continuation,
            AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(third_run_id.to_string()).expect("run ref"),
                gate_ref: AuthGateRef::new(gate_ref).expect("gate ref"),
            }
        );

        let Json(reconfigure) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(WebUiAuthenticatedCaller::new(
                tenant_id, user_id, None, None,
            )),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack replacement".to_string(),
                scopes: vec![],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(InvocationId::new().to_string()),
            }),
        )
        .await
        .expect("an active Slack connection may start one replacement OAuth flow");
        let replacement_epoch = SlackConnectionEpoch::new(reconfigure.flow_id);
        assert_ne!(replacement_epoch, epoch);
        assert_eq!(
            lifecycle_store
                .connection_state(&owner)
                .await
                .expect("replacement lifecycle state"),
            Some((replacement_epoch, SlackConnectionState::Connecting))
        );
        assert_eq!(
            shared
                .accounts_for_owner(&callback_state.scope().to_credential_owner())
                .await
                .expect("existing account remains available")
                .len(),
            1,
            "starting reconfigure must not remove the working account"
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
    async fn provider_denials_preserve_response_and_attempt_failing_terminal_hook_once() {
        FAILING_TERMINAL_HOOK_CALLS.store(0, Ordering::SeqCst);
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let secret_store = Arc::new(InMemorySecretStore::new());
        let secret_store_for_provider: Arc<dyn SecretStore> = secret_store.clone();
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
            RebornProductAuthServices::from_shared(
                shared.clone(),
                Arc::new(RecordingDispatcher::default()),
            )
            .with_flow_record_source(shared)
            .with_oauth_gate_registry(Arc::new(OAuthGateProviderRegistry::new(vec![google_gate]))),
        );
        let state = ProductAuthRouteState::new(
            product_auth.clone(),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
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
                &TurnScope::new(
                    TenantId::new("tenant-alpha").expect("tenant"),
                    None,
                    None,
                    ThreadId::new("thread-alpha").expect("thread"),
                ),
                &UserId::new("user-alpha").expect("user"),
                TurnRunId::new(),
                "gate:gmail-auth",
                &requirements,
            )
            .await
            .expect("challenge lookup")
            .expect("google oauth challenge");
        let state_value = Url::parse(
            challenge
                .authorization_url
                .expect("authorization url")
                .as_str(),
        )
        .expect("authorization url")
        .query_pairs()
        .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
        .expect("oauth state");
        let flow_id = OAuthCallbackState::decode(OAuthCallbackStateKind::GOOGLE, &state_value)
            .expect("decode callback state")
            .flow_id();
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let uri = format!(
            "{GOOGLE_OAUTH_CALLBACK_PATH}?state={encoded_state}&code=google-auth-code&scope="
        )
        .parse::<Uri>()
        .expect("callback uri");
        let raw_query = uri.query().map(str::to_string);
        let descriptor = OAuthCallbackDescriptor {
            state_kind: OAuthCallbackStateKind::GOOGLE,
            provider_id: GOOGLE_PROVIDER_ID,
            scope_resolution: CallbackScopeResolution::ValidateEchoedIncludesRequested,
            identity_hook: no_identity_hook,
            on_terminal_failure: Some(failing_terminal_hook),
        };
        let mut known_flow_id = None;

        let error = oauth_provider_callback_attempt(
            state,
            &descriptor,
            raw_query,
            uri,
            &HeaderMap::new(),
            &mut known_flow_id,
        )
        .await
        .expect_err("empty granted scope must preserve the provider-denied response");

        assert_eq!(error.body.code, AuthErrorCode::ProviderDenied);
        assert_eq!(known_flow_id, Some(flow_id));
        assert_eq!(
            FAILING_TERMINAL_HOOK_CALLS.load(Ordering::SeqCst),
            1,
            "terminal cleanup is retryable but must be attempted only once per callback"
        );

        let second_challenge = product_auth
            .challenge_for_gate(
                &TurnScope::new(
                    TenantId::new("tenant-alpha").expect("tenant"),
                    None,
                    None,
                    ThreadId::new("thread-beta").expect("thread"),
                ),
                &UserId::new("user-alpha").expect("user"),
                TurnRunId::new(),
                "gate:gmail-auth-retry",
                &requirements,
            )
            .await
            .expect("second challenge lookup")
            .expect("second google oauth challenge");
        let second_state_value = Url::parse(
            second_challenge
                .authorization_url
                .expect("authorization url")
                .as_str(),
        )
        .expect("authorization url")
        .query_pairs()
        .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
        .expect("oauth state");
        let encoded_state =
            url::form_urlencoded::byte_serialize(second_state_value.as_bytes()).collect::<String>();
        let uri = format!("{GOOGLE_OAUTH_CALLBACK_PATH}?state={encoded_state}&error=access_denied")
            .parse::<Uri>()
            .expect("callback uri");
        let raw_query = uri.query().map(str::to_string);
        let state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
        let mut second_known_flow_id = None;

        let error = oauth_provider_callback_attempt(
            state,
            &descriptor,
            raw_query,
            uri,
            &HeaderMap::new(),
            &mut second_known_flow_id,
        )
        .await
        .expect_err("explicit provider denial must preserve the provider-denied response");

        assert_eq!(error.body.code, AuthErrorCode::ProviderDenied);
        assert_eq!(
            FAILING_TERMINAL_HOOK_CALLS.load(Ordering::SeqCst),
            2,
            "each provider-denied callback must attempt terminal cleanup exactly once"
        );
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
        .with_test_installed_extension_lookup()
        .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
        .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
            binding_service,
            Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                SlackPersonalConnectionScope { installation_id },
            ))),
            binding_store,
            Arc::new(TestSlackLifecycleStore::default()),
        ));

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
        assert_eq!(
            start_response.continuation,
            AuthContinuationRef::LifecycleActivation {
                package_ref: ironclaw_auth::LifecyclePackageRef::new("slack")
                    .expect("lifecycle package ref"),
            }
        );

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
    async fn slack_personal_oauth_start_failure_before_publish_creates_no_lifecycle_epoch() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let product_auth = Arc::new(RebornProductAuthServices::new(
            Arc::new(FailingCompletionFlowManager {
                inner: shared.clone(),
                fail_create: true,
            }),
            shared.clone(),
            shared.clone(),
            shared.clone(),
            shared.clone(),
            shared.clone(),
            Arc::new(RecordingDispatcher::default()),
        ));
        let tenant_id = TenantId::new("tenant-alpha").expect("tenant");
        let user_id = UserId::new("user-alpha").expect("user");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation");
        let binding_store = Arc::new(RecordingBindingStore::default());
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant_id.clone(),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            binding_store.clone(),
        ));
        let lifecycle_store = Arc::new(TestSlackLifecycleStore::default());
        let state = ProductAuthRouteState::new(product_auth.clone(), tenant_id.clone(), None, None)
            .with_test_installed_extension_lookup()
            .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
            .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
                binding_service,
                Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                    SlackPersonalConnectionScope { installation_id },
                ))),
                binding_store,
                lifecycle_store.clone(),
            ));

        let error = extension_oauth_start_handler(
            State(state),
            Extension(WebUiAuthenticatedCaller::new(
                tenant_id, user_id, None, None,
            )),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack".to_string(),
                scopes: Vec::new(),
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(InvocationId::new().to_string()),
            }),
        )
        .await
        .expect_err("failed durable flow publication must be surfaced");

        assert_eq!(error.status, StatusCode::CONFLICT);
        assert_eq!(error.body.code, AuthErrorCode::CredentialMissing);
        assert!(!error.body.retryable);
        assert_eq!(
            lifecycle_store
                .entries
                .lock()
                .expect("lifecycle entries lock")
                .len(),
            0,
            "flow publication failure happens before lifecycle reservation"
        );
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
        .with_test_installed_extension_lookup()
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
    #[test]
    fn slack_lifecycle_start_conflicts_do_not_claim_another_user_owns_the_identity() {
        for error in [
            SlackUserBindingLifecycleError::ConnectionInProgress,
            SlackUserBindingLifecycleError::DisconnectInProgress,
        ] {
            let failure = slack_lifecycle_start_failure(error);
            assert_eq!(failure.status, StatusCode::CONFLICT);
            assert_eq!(failure.body.code, AuthErrorCode::ConnectionConflict);
            assert_ne!(
                failure.body.code,
                AuthErrorCode::ProviderIdentityAlreadyConnected
            );
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_terminal_cleanup_without_binding_authority_stays_retryable() {
        let product_auth = Arc::new(RebornProductAuthServices::from_shared(
            Arc::new(InMemoryAuthProductServices::new()),
            Arc::new(RecordingDispatcher::default()),
        ));
        let tenant_id = TenantId::new("tenant-alpha").expect("tenant");
        let state = ProductAuthRouteState::new(product_auth, tenant_id.clone(), None, None);
        let callback_scope = AuthProductScope::new(
            ResourceScope {
                tenant_id,
                user_id: UserId::new("user-alpha").expect("user"),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            AuthSurface::Callback,
        );

        let error = run_terminal_failure_hook(
            &state,
            &SLACK_PERSONAL_CALLBACK_DESCRIPTOR,
            &callback_scope,
            AuthFlowId::new(),
            RebornOAuthCallbackFailureStage::ContinuationSideEffect,
        )
        .await
        .expect_err("missing Slack cleanup authority must keep status retryable");

        assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.body.code, AuthErrorCode::BackendUnavailable);
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_personal_oauth_denial_abandons_the_epoch_owner_after_setup_drift() {
        let product_auth = Arc::new(RebornProductAuthServices::from_shared(
            Arc::new(InMemoryAuthProductServices::new()),
            Arc::new(RecordingDispatcher::default()),
        ));
        let tenant_id = TenantId::new("tenant-alpha").expect("tenant");
        let user_id = UserId::new("user-alpha").expect("user");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation");
        let binding_store = Arc::new(RecordingBindingStore::default());
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant_id.clone(),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            binding_store.clone(),
        ));
        let lifecycle_store = Arc::new(TestSlackLifecycleStore::default());
        let state = ProductAuthRouteState::new(product_auth, tenant_id.clone(), None, None)
            .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
                binding_service,
                Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                    SlackPersonalConnectionScope {
                        installation_id: AdapterInstallationId::new("install-drifted")
                            .expect("drifted installation"),
                    },
                ))),
                binding_store,
                lifecycle_store.clone(),
            ));
        let flow_id = AuthFlowId::new();
        let epoch = SlackConnectionEpoch::new(flow_id);
        let owner = SlackConnectionOwner::new(tenant_id.clone(), user_id.clone(), installation_id);
        lifecycle_store
            .begin_connection(&owner, epoch, Utc::now() + ChronoDuration::minutes(5))
            .await
            .expect("connection begins");
        let callback_scope = AuthProductScope::new(
            ResourceScope {
                tenant_id: tenant_id.clone(),
                user_id: user_id.clone(),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            AuthSurface::Callback,
        );

        run_terminal_failure_hook(
            &state,
            &SLACK_PERSONAL_CALLBACK_DESCRIPTOR,
            &callback_scope,
            flow_id,
            RebornOAuthCallbackFailureStage::Terminal,
        )
        .await
        .expect("terminal Slack cleanup");

        assert_eq!(
            lifecycle_store
                .connection_owner_for_epoch(&tenant_id, &user_id, epoch)
                .await
                .expect("owner lookup after denial"),
            None,
            "denial must abandon the owner captured before setup drift"
        );
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
        let lifecycle_store = Arc::new(TestSlackLifecycleStore::default());
        let state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_test_installed_extension_lookup()
        .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
        .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
            binding_service.clone(),
            Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                SlackPersonalConnectionScope {
                    installation_id: installation_id.clone(),
                },
            ))),
            binding_store.clone(),
            lifecycle_store.clone(),
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
        // Setup may change while the provider popup is open. The callback must
        // target the owner captured by this flow's epoch, not re-resolve the
        // currently configured installation.
        let state = state.with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
            binding_service,
            Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                SlackPersonalConnectionScope {
                    installation_id: AdapterInstallationId::new("install-drifted")
                        .expect("drifted installation"),
                },
            ))),
            binding_store.clone(),
            lifecycle_store,
        ));
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
    async fn slack_personal_terminal_callback_failures_allow_immediate_retry() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let provider_identity = OAuthProviderIdentity::new(
            "U123",
            Some("T123".to_string()),
            None,
            Some("A123".to_string()),
        )
        .expect("provider identity");
        let provider_client = Arc::new(SlackIdentityProviderClient::new(provider_identity));
        provider_client.fail_exchange_with_backend_unavailable();
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(
                shared.clone(),
                Arc::new(RecordingDispatcher::default()),
            )
            .with_flow_record_source(shared)
            .with_provider_client(provider_client),
        );
        let tenant_id = TenantId::new("tenant-alpha").expect("tenant");
        let user_id = UserId::new("user-alpha").expect("user");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation");
        let binding_store = Arc::new(RecordingBindingStore::default());
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant_id.clone(),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            binding_store.clone(),
        ));
        let lifecycle_store = Arc::new(TestSlackLifecycleStore::default());
        let state = ProductAuthRouteState::new(product_auth.clone(), tenant_id.clone(), None, None)
            .with_test_installed_extension_lookup()
            .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
            .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
                binding_service,
                Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                    SlackPersonalConnectionScope { installation_id },
                ))),
                binding_store,
                lifecycle_store.clone(),
            ));
        let caller = WebUiAuthenticatedCaller::new(tenant_id.clone(), user_id.clone(), None, None);
        let Json(first_start) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(caller.clone()),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack".to_string(),
                scopes: vec![],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(InvocationId::new().to_string()),
            }),
        )
        .await
        .expect("first Slack OAuth starts");
        let state_value = Url::parse(first_start.authorization_url.as_str())
            .expect("authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("oauth state");
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let uri = format!("{SLACK_PERSONAL_OAUTH_CALLBACK_PATH}?state={encoded_state}")
            .parse::<Uri>()
            .expect("callback uri");

        let error = slack_personal_oauth_callback_handler(
            State(state.clone()),
            RawQuery(uri.query().map(str::to_string)), // safety: URI query parsing, not a database query.
            uri,
            HeaderMap::new(),
        )
        .await
        .expect_err("known-flow callback without a code must fail");
        assert_eq!(error.body.code, AuthErrorCode::MalformedCallback);
        let first_callback_state =
            OAuthCallbackState::decode(OAuthCallbackStateKind::SLACK_PERSONAL, &state_value)
                .expect("decode first callback state");
        assert_eq!(
            product_auth
                .flow_record_for_status(first_callback_state.scope(), first_start.flow_id)
                .await
                .expect("load malformed callback flow")
                .status,
            AuthFlowStatus::Failed,
            "known malformed callback must durably terminalize the flow"
        );
        let first_epoch = SlackConnectionEpoch::new(first_start.flow_id);
        assert_eq!(
            lifecycle_store
                .connection_owner_for_epoch(&tenant_id, &user_id, first_epoch)
                .await
                .expect("lifecycle lookup"),
            None,
            "terminal malformed callback must abandon the connecting epoch"
        );

        let Json(second_start) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(caller.clone()),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack retry".to_string(),
                scopes: vec![],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(InvocationId::new().to_string()),
            }),
        )
        .await
        .expect("malformed terminal callback must not block immediate reconnect");
        let state_value = Url::parse(second_start.authorization_url.as_str())
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
            State(state.clone()),
            RawQuery(uri.query().map(str::to_string)), // safety: URI query parsing, not a database query.
            uri,
            HeaderMap::new(),
        )
        .await
        .expect_err("provider backend failure must surface");
        assert_eq!(error.body.code, AuthErrorCode::BackendUnavailable);
        let second_epoch = SlackConnectionEpoch::new(second_start.flow_id);
        assert_eq!(
            lifecycle_store
                .connection_owner_for_epoch(&tenant_id, &user_id, second_epoch)
                .await
                .expect("lifecycle lookup"),
            None,
            "terminal provider failure must abandon the connecting epoch"
        );

        let Json(third_start) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(caller.clone()),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack pkce retry".to_string(),
                scopes: vec![],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(InvocationId::new().to_string()),
            }),
        )
        .await
        .expect("terminal provider failure must not block immediate reconnect");
        state.remove_pkce_verifier(third_start.flow_id);
        let state_value = Url::parse(third_start.authorization_url.as_str())
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
            State(state.clone()),
            RawQuery(uri.query().map(str::to_string)), // safety: URI query parsing, not a database query.
            uri,
            HeaderMap::new(),
        )
        .await
        .expect_err("known callback without PKCE must fail");
        assert_eq!(error.body.code, AuthErrorCode::UnknownOrExpiredFlow);
        let third_callback_state =
            OAuthCallbackState::decode(OAuthCallbackStateKind::SLACK_PERSONAL, &state_value)
                .expect("decode third callback state");
        assert_eq!(
            product_auth
                .flow_record_for_status(third_callback_state.scope(), third_start.flow_id)
                .await
                .expect("load missing-PKCE callback flow")
                .status,
            AuthFlowStatus::Failed,
            "missing one-shot PKCE material must durably terminalize the known flow"
        );
        let third_epoch = SlackConnectionEpoch::new(third_start.flow_id);
        assert_eq!(
            lifecycle_store
                .connection_owner_for_epoch(&tenant_id, &user_id, third_epoch)
                .await
                .expect("lifecycle lookup"),
            None,
            "missing PKCE must abandon the connecting epoch"
        );

        let _ = extension_oauth_start_handler(
            State(state),
            Extension(caller),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack final retry".to_string(),
                scopes: vec![],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(InvocationId::new().to_string()),
            }),
        )
        .await
        .expect("missing PKCE must not block immediate reconnect");
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
        .with_test_installed_extension_lookup()
        .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
        .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
            binding_service,
            Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                SlackPersonalConnectionScope { installation_id },
            ))),
            binding_store.clone(),
            Arc::new(TestSlackLifecycleStore::default()),
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
            State(state.clone()),
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
    async fn slack_personal_oauth_callback_renders_duplicate_slack_identity_failure() {
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
        provider_client.fail_cleanup();
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), dispatcher)
                .with_flow_record_source(shared.clone())
                .with_provider_client(provider_client.clone()),
        );
        let failing_binding_store = Arc::new(FailingBindingStore);
        let rollback_store = Arc::new(RecordingBindingStore::default());
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation");
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            failing_binding_store,
        ));
        let state = ProductAuthRouteState::new(
            product_auth.clone(),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_test_installed_extension_lookup()
        .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
        .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
            binding_service,
            Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                SlackPersonalConnectionScope { installation_id },
            ))),
            rollback_store,
            Arc::new(TestSlackLifecycleStore::default()),
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
        let flow_id = start_response.flow_id;
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
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            "text/html,application/xhtml+xml".parse().expect("accept"),
        );

        let response = slack_personal_oauth_callback_handler(
            State(state),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            headers,
        )
        .await
        .expect("browser callback failures render an HTML completion page");

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let body = String::from_utf8(body.to_vec()).expect("utf8 body");
        assert!(
            body.contains(
                "This provider account is already connected to another Reborn user. Disconnect it from that user, then try again."
            ),
            "duplicate identity failure should tell the user how to fix it: {body}"
        );
        assert!(body.contains(r#""status":"failed""#));

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
        );
        let cleanup_account_id = CredentialAccountId::from_uuid(flow_id.as_uuid());
        let cleanup_account = shared
            .accounts_for_owner(&owner_scope)
            .await
            .expect("list rejected callback cleanup account")
            .into_iter()
            .find(|account| account.id == cleanup_account_id)
            .expect("failed provider cleanup must retain exchanged handles");
        assert_eq!(cleanup_account.status, CredentialAccountStatus::Revoked);
        assert!(cleanup_account.access_secret.is_some());
        assert!(cleanup_account.refresh_secret.is_some());

        product_auth
            .cleanup_credentials_for_lifecycle(ironclaw_auth::SecretCleanupRequest {
                scope: owner_scope.clone(),
                extension_id: ExtensionId::new("slack").expect("extension"),
                provider: Some(
                    AuthProviderId::new(SLACK_PERSONAL_PROVIDER_ID).expect("Slack provider"),
                ),
                action: ironclaw_auth::SecretCleanupAction::Uninstall,
            })
            .await
            .expect("lifecycle cleanup must retry retained exchange deletion");
        let retried_cleanup_account = shared
            .accounts_for_owner(&owner_scope)
            .await
            .expect("list retried cleanup account")
            .into_iter()
            .find(|account| account.id == cleanup_account_id)
            .expect("revoked cleanup account remains as an empty tombstone");
        assert!(retried_cleanup_account.access_secret.is_none());
        assert!(retried_cleanup_account.refresh_secret.is_none());
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
        .with_test_installed_extension_lookup()
        .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
        .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
            binding_service,
            Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                SlackPersonalConnectionScope { installation_id },
            ))),
            binding_store.clone(),
            Arc::new(TestSlackLifecycleStore::default()),
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
            State(state.clone()),
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

    #[cfg(feature = "slack-v2-host-beta")]
    struct RejectingContinuationDispatcher;

    #[cfg(feature = "slack-v2-host-beta")]
    #[async_trait]
    impl RebornAuthContinuationDispatcher for RejectingContinuationDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            _event: ironclaw_auth::AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            Err(AuthProductError::BackendUnavailable)
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[derive(Default)]
    struct RejectingSecondContinuationDispatcher {
        calls: AtomicUsize,
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[async_trait]
    impl RebornAuthContinuationDispatcher for RejectingSecondContinuationDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            _event: ironclaw_auth::AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(())
            } else {
                Err(AuthProductError::BackendUnavailable)
            }
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[derive(Clone)]
    struct SlackIdentityProviderClient {
        provider_identity: OAuthProviderIdentity,
        calls: Arc<AtomicUsize>,
        cleanup_calls: Arc<AtomicUsize>,
        fail_exchange_backend_unavailable: Arc<AtomicBool>,
        fail_cleanup: Arc<AtomicBool>,
    }

    #[cfg(feature = "slack-v2-host-beta")]
    impl SlackIdentityProviderClient {
        fn new(provider_identity: OAuthProviderIdentity) -> Self {
            Self {
                provider_identity,
                calls: Arc::new(AtomicUsize::new(0)),
                cleanup_calls: Arc::new(AtomicUsize::new(0)),
                fail_exchange_backend_unavailable: Arc::new(AtomicBool::new(false)),
                fail_cleanup: Arc::new(AtomicBool::new(false)),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn cleanup_calls(&self) -> usize {
            self.cleanup_calls.load(Ordering::SeqCst)
        }

        fn fail_exchange_with_backend_unavailable(&self) {
            self.fail_exchange_backend_unavailable
                .store(true, Ordering::SeqCst);
        }

        fn fail_cleanup(&self) {
            self.fail_cleanup.store(true, Ordering::SeqCst);
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[async_trait]
    impl AuthProviderClient for SlackIdentityProviderClient {
        async fn exchange_callback(
            &self,
            _context: OAuthProviderExchangeContext,
            request: OAuthProviderCallbackRequest,
        ) -> Result<OAuthProviderExchange, AuthProductError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self
                .fail_exchange_backend_unavailable
                .load(Ordering::SeqCst)
            {
                return Err(AuthProductError::BackendUnavailable);
            }
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
            if self.fail_cleanup.load(Ordering::SeqCst) {
                Err(AuthProductError::BackendUnavailable)
            } else {
                Ok(())
            }
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[derive(Default)]
    struct RecordingBindingStore {
        bindings: Arc<Mutex<Vec<RebornUserIdentityBinding>>>,
        binding_epochs: Arc<Mutex<Vec<Option<SlackConnectionEpoch>>>>,
        fail_delete_once: Arc<Mutex<bool>>,
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[derive(Debug, Default)]
    struct TestSlackLifecycleStore {
        entries: Mutex<Vec<TestSlackLifecycleEntry>>,
        fail_next_abandon: AtomicBool,
        transition_after_next_state_read: Mutex<Option<SlackConnectionState>>,
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[derive(Debug)]
    struct TestSlackLifecycleEntry {
        epoch: SlackConnectionEpoch,
        owner: SlackConnectionOwner,
        state: SlackConnectionState,
        pending: Option<(SlackConnectionEpoch, ironclaw_auth::Timestamp)>,
        cleanup_selector: Option<SlackConnectionCleanupSelector>,
    }

    #[cfg(feature = "slack-v2-host-beta")]
    impl TestSlackLifecycleStore {
        fn activate(
            &self,
            owner: &SlackConnectionOwner,
            epoch: SlackConnectionEpoch,
        ) -> Option<SlackConnectionEpoch> {
            let mut entries = self.entries.lock().expect("lifecycle entries lock");
            let entry = entries
                .iter_mut()
                .find(|entry| entry.owner == *owner)
                .expect("connecting lifecycle owner");
            if entry.state == SlackConnectionState::Active
                && entry
                    .pending
                    .is_some_and(|(pending_epoch, _)| pending_epoch == epoch)
            {
                let previous_epoch = entry.epoch;
                entry.epoch = epoch;
                entry.pending = None;
                Some(previous_epoch)
            } else {
                assert_eq!(entry.epoch, epoch);
                assert_eq!(entry.state, SlackConnectionState::Connecting);
                entry.state = SlackConnectionState::Active;
                None
            }
        }

        fn restore_after_failed_reconfigure(
            &self,
            owner: &SlackConnectionOwner,
            failed_epoch: SlackConnectionEpoch,
            previous_epoch: SlackConnectionEpoch,
        ) {
            let mut entries = self.entries.lock().expect("lifecycle entries lock");
            let entry = entries
                .iter_mut()
                .find(|entry| entry.owner == *owner)
                .expect("active lifecycle owner");
            if entry.state == SlackConnectionState::Active && entry.epoch == failed_epoch {
                entry.epoch = previous_epoch;
                entry.pending = None;
            }
        }

        fn transition_after_next_state_read(&self, state: SlackConnectionState) {
            *self
                .transition_after_next_state_read
                .lock()
                .expect("lifecycle transition lock") = Some(state);
        }

        fn set_state(
            &self,
            owner: &SlackConnectionOwner,
            epoch: SlackConnectionEpoch,
            state: SlackConnectionState,
        ) {
            let mut entries = self.entries.lock().expect("lifecycle entries lock");
            let entry = entries
                .iter_mut()
                .find(|entry| entry.owner == *owner && entry.epoch == epoch)
                .expect("lifecycle owner");
            entry.state = state;
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[async_trait]
    impl SlackUserBindingLifecycleStore for TestSlackLifecycleStore {
        async fn begin_connection(
            &self,
            owner: &SlackConnectionOwner,
            epoch: SlackConnectionEpoch,
            expires_at: ironclaw_auth::Timestamp,
        ) -> Result<(), SlackUserBindingLifecycleError> {
            let mut entries = self.entries.lock().expect("lifecycle entries lock");
            if let Some(entry) = entries.iter_mut().find(|entry| entry.owner == *owner) {
                match entry.state {
                    SlackConnectionState::Connecting if entry.epoch == epoch => return Ok(()),
                    SlackConnectionState::Connecting => {
                        return Err(SlackUserBindingLifecycleError::ConnectionInProgress);
                    }
                    SlackConnectionState::Active => match entry.pending {
                        Some((pending_epoch, _)) if pending_epoch == epoch => return Ok(()),
                        Some((_, pending_expiry)) if pending_expiry > Utc::now() => {
                            return Err(SlackUserBindingLifecycleError::ConnectionInProgress);
                        }
                        Some(_) | None => {
                            entry.pending = Some((epoch, expires_at));
                            return Ok(());
                        }
                    },
                    SlackConnectionState::Disconnecting => {
                        return Err(SlackUserBindingLifecycleError::DisconnectInProgress);
                    }
                    SlackConnectionState::Disconnected => {
                        entry.epoch = epoch;
                        entry.state = SlackConnectionState::Connecting;
                        entry.pending = None;
                        entry.cleanup_selector = None;
                        return Ok(());
                    }
                }
            }
            entries.push(TestSlackLifecycleEntry {
                epoch,
                owner: owner.clone(),
                state: SlackConnectionState::Connecting,
                pending: None,
                cleanup_selector: None,
            });
            Ok(())
        }

        async fn connection_state(
            &self,
            owner: &SlackConnectionOwner,
        ) -> Result<
            Option<(SlackConnectionEpoch, SlackConnectionState)>,
            SlackUserBindingLifecycleError,
        > {
            let mut entries = self.entries.lock().expect("lifecycle entries lock");
            let current = entries
                .iter()
                .find(|entry| entry.owner == *owner)
                .map(|entry| match (entry.state, entry.pending) {
                    (SlackConnectionState::Active, Some((pending_epoch, _))) => {
                        (pending_epoch, SlackConnectionState::Connecting)
                    }
                    _ => (entry.epoch, entry.state),
                });
            if let Some(next_state) = self
                .transition_after_next_state_read
                .lock()
                .expect("lifecycle transition lock")
                .take()
                && let Some(entry) = entries.iter_mut().find(|entry| entry.owner == *owner)
            {
                entry.state = next_state;
            }
            Ok(current)
        }

        async fn connection_owner_for_epoch(
            &self,
            tenant_id: &TenantId,
            user_id: &UserId,
            epoch: SlackConnectionEpoch,
        ) -> Result<Option<SlackConnectionOwner>, SlackUserBindingLifecycleError> {
            Ok(self
                .entries
                .lock()
                .expect("lifecycle entries lock")
                .iter()
                .rev()
                .find(|entry| {
                    (entry.epoch == epoch
                        || (entry.state == SlackConnectionState::Active
                            && entry
                                .pending
                                .is_some_and(|(pending_epoch, _)| pending_epoch == epoch)))
                        && entry.state != SlackConnectionState::Disconnected
                        && entry.owner.tenant_id() == tenant_id
                        && entry.owner.user_id() == user_id
                })
                .map(|entry| entry.owner.clone()))
        }

        async fn connection_owners_for_user(
            &self,
            tenant_id: &TenantId,
            user_id: &UserId,
        ) -> Result<Vec<SlackConnectionOwner>, SlackUserBindingLifecycleError> {
            Ok(self
                .entries
                .lock()
                .expect("lifecycle entries lock")
                .iter()
                .filter(|entry| {
                    entry.state != SlackConnectionState::Disconnected
                        && entry.owner.tenant_id() == tenant_id
                        && entry.owner.user_id() == user_id
                })
                .map(|entry| entry.owner.clone())
                .collect())
        }

        async fn begin_disconnect(
            &self,
            owner: &SlackConnectionOwner,
        ) -> Result<SlackDisconnectFence, SlackUserBindingLifecycleError> {
            let mut entries = self.entries.lock().expect("lifecycle entries lock");
            if let Some(entry) = entries.iter_mut().find(|entry| entry.owner == *owner) {
                if entry.state == SlackConnectionState::Disconnecting {
                    return Ok(SlackDisconnectFence::new(
                        entry.epoch,
                        entry
                            .cleanup_selector
                            .unwrap_or(SlackConnectionCleanupSelector::Epoch(entry.epoch)),
                    ));
                }
                if matches!(
                    entry.state,
                    SlackConnectionState::Connecting | SlackConnectionState::Active
                ) {
                    entry.state = SlackConnectionState::Disconnecting;
                    entry.pending = None;
                    entry.cleanup_selector =
                        Some(SlackConnectionCleanupSelector::Epoch(entry.epoch));
                    return Ok(SlackDisconnectFence::new(
                        entry.epoch,
                        SlackConnectionCleanupSelector::Epoch(entry.epoch),
                    ));
                }
                let fence_epoch = SlackConnectionEpoch::new(AuthFlowId::new());
                entry.epoch = fence_epoch;
                entry.state = SlackConnectionState::Disconnecting;
                entry.pending = None;
                entry.cleanup_selector = Some(SlackConnectionCleanupSelector::AllOwned);
                return Ok(SlackDisconnectFence::new(
                    fence_epoch,
                    SlackConnectionCleanupSelector::AllOwned,
                ));
            }
            let fence_epoch = SlackConnectionEpoch::new(AuthFlowId::new());
            entries.push(TestSlackLifecycleEntry {
                epoch: fence_epoch,
                owner: owner.clone(),
                state: SlackConnectionState::Disconnecting,
                pending: None,
                cleanup_selector: Some(SlackConnectionCleanupSelector::AllOwned),
            });
            Ok(SlackDisconnectFence::new(
                fence_epoch,
                SlackConnectionCleanupSelector::AllOwned,
            ))
        }

        async fn complete_disconnect(
            &self,
            owner: &SlackConnectionOwner,
            epoch: SlackConnectionEpoch,
        ) -> Result<(), SlackUserBindingLifecycleError> {
            let mut entries = self.entries.lock().expect("lifecycle entries lock");
            let Some(entry) = entries
                .iter_mut()
                .find(|entry| entry.owner == *owner && entry.epoch == epoch)
            else {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            };
            if entry.state != SlackConnectionState::Disconnecting {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            }
            entry.state = SlackConnectionState::Disconnected;
            entry.pending = None;
            entry.cleanup_selector = None;
            Ok(())
        }

        async fn begin_failed_connection_cleanup(
            &self,
            owner: &SlackConnectionOwner,
            epoch: SlackConnectionEpoch,
        ) -> Result<(), SlackUserBindingLifecycleError> {
            let mut entries = self.entries.lock().expect("lifecycle entries lock");
            let Some(entry) = entries.iter_mut().find(|entry| entry.owner == *owner) else {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            };
            if entry.state == SlackConnectionState::Active
                && entry
                    .pending
                    .is_some_and(|(pending_epoch, _)| pending_epoch == epoch)
            {
                return Ok(());
            }
            if entry.epoch != epoch {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            }
            if entry.state != SlackConnectionState::Disconnected {
                entry.state = SlackConnectionState::Disconnecting;
                entry.cleanup_selector = Some(SlackConnectionCleanupSelector::Epoch(epoch));
            }
            Ok(())
        }

        async fn complete_failed_connection_cleanup(
            &self,
            owner: &SlackConnectionOwner,
            epoch: SlackConnectionEpoch,
        ) -> Result<(), SlackUserBindingLifecycleError> {
            let mut entries = self.entries.lock().expect("lifecycle entries lock");
            let Some(entry) = entries.iter_mut().find(|entry| entry.owner == *owner) else {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            };
            if entry.state == SlackConnectionState::Active
                && entry
                    .pending
                    .is_some_and(|(pending_epoch, _)| pending_epoch == epoch)
            {
                entry.pending = None;
                return Ok(());
            }
            if entry.epoch != epoch {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            }
            entry.state = SlackConnectionState::Disconnected;
            entry.pending = None;
            entry.cleanup_selector = None;
            Ok(())
        }

        async fn abandon_connection(
            &self,
            owner: &SlackConnectionOwner,
            epoch: SlackConnectionEpoch,
        ) -> Result<(), SlackUserBindingLifecycleError> {
            if self.fail_next_abandon.swap(false, Ordering::SeqCst) {
                return Err(SlackUserBindingLifecycleError::Backend(
                    "scripted abandon failure".to_string(),
                ));
            }
            let mut entries = self.entries.lock().expect("lifecycle entries lock");
            if let Some(entry) = entries.iter_mut().find(|entry| entry.owner == *owner) {
                if entry.state == SlackConnectionState::Active
                    && entry
                        .pending
                        .is_some_and(|(pending_epoch, _)| pending_epoch == epoch)
                {
                    entry.pending = None;
                } else if entry.epoch == epoch && entry.state != SlackConnectionState::Disconnecting
                {
                    entry.state = SlackConnectionState::Disconnected;
                    entry.pending = None;
                    entry.cleanup_selector = None;
                }
            }
            Ok(())
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    impl RecordingBindingStore {
        fn bindings(&self) -> Vec<RebornUserIdentityBinding> {
            self.bindings.lock().expect("binding store lock").clone()
        }

        fn fail_next_delete(&self) {
            *self.fail_delete_once.lock().expect("delete failure lock") = true;
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
            self.binding_epochs
                .lock()
                .expect("binding epoch lock")
                .push(None);
            Ok(())
        }

        async fn bind_user_identity_for_epoch(
            &self,
            binding: RebornUserIdentityBinding,
            epoch: SlackConnectionEpoch,
        ) -> Result<
            crate::slack::slack_personal_binding::SlackUserIdentityBindingRollback,
            RebornUserIdentityBindingError,
        > {
            let previous = {
                let mut bindings = self.bindings.lock().expect("binding store lock");
                let mut binding_epochs = self.binding_epochs.lock().expect("binding epoch lock");
                let existing_index = bindings.iter().position(|candidate| {
                    candidate.provider == binding.provider
                        && candidate.provider_user_id == binding.provider_user_id
                        && candidate.user_id == binding.user_id
                });
                if let Some(index) = existing_index {
                    let previous = (bindings[index].clone(), binding_epochs[index]);
                    bindings[index] = binding.clone();
                    binding_epochs[index] = Some(epoch);
                    Some(previous)
                } else {
                    bindings.push(binding.clone());
                    binding_epochs.push(Some(epoch));
                    None
                }
            };
            let bindings = Arc::clone(&self.bindings);
            let binding_epochs = Arc::clone(&self.binding_epochs);
            let fail_delete_once = Arc::clone(&self.fail_delete_once);
            Ok(
                crate::slack::slack_personal_binding::SlackUserIdentityBindingRollback::new(
                    async move {
                        let mut fail_delete = fail_delete_once.lock().expect("delete failure lock");
                        if *fail_delete {
                            *fail_delete = false;
                            return;
                        }
                        drop(fail_delete);
                        let mut bindings = bindings.lock().expect("binding store lock");
                        let mut binding_epochs = binding_epochs.lock().expect("binding epoch lock");
                        let current_index = bindings.iter().position(|candidate| {
                            candidate.provider == binding.provider
                                && candidate.provider_user_id == binding.provider_user_id
                                && candidate.user_id == binding.user_id
                        });
                        if let Some(index) = current_index
                            && binding_epochs[index] == Some(epoch)
                        {
                            if let Some((previous_binding, previous_epoch)) = previous {
                                bindings[index] = previous_binding;
                                binding_epochs[index] = previous_epoch;
                            } else {
                                bindings.remove(index);
                                binding_epochs.remove(index);
                            }
                        }
                    },
                ),
            )
        }
    }

    /// Test seam mirroring the production filesystem store's atomic
    /// identity-bind + lifecycle activation write.
    #[cfg(feature = "slack-v2-host-beta")]
    struct ActivatingBindingStore {
        inner: Arc<RecordingBindingStore>,
        lifecycle_store: Arc<TestSlackLifecycleStore>,
        owner: SlackConnectionOwner,
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[async_trait]
    impl RebornUserIdentityBindingStore for ActivatingBindingStore {
        async fn bind_user_identity(
            &self,
            binding: RebornUserIdentityBinding,
        ) -> Result<(), RebornUserIdentityBindingError> {
            self.inner.bind_user_identity(binding).await
        }

        async fn bind_user_identity_for_epoch(
            &self,
            binding: RebornUserIdentityBinding,
            epoch: SlackConnectionEpoch,
        ) -> Result<
            crate::slack::slack_personal_binding::SlackUserIdentityBindingRollback,
            RebornUserIdentityBindingError,
        > {
            let rollback = self
                .inner
                .bind_user_identity_for_epoch(binding, epoch)
                .await?;
            let previous_epoch = self.lifecycle_store.activate(&self.owner, epoch);
            let lifecycle_store = Arc::clone(&self.lifecycle_store);
            let owner = self.owner.clone();
            Ok(
                crate::slack::slack_personal_binding::SlackUserIdentityBindingRollback::new(
                    async move {
                        rollback.into_future().await;
                        if let Some(previous_epoch) = previous_epoch {
                            lifecycle_store.restore_after_failed_reconfigure(
                                &owner,
                                epoch,
                                previous_epoch,
                            );
                        } else {
                            let _ = lifecycle_store.abandon_connection(&owner, epoch).await;
                        }
                    },
                ),
            )
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    struct FailingBindingStore;

    #[cfg(feature = "slack-v2-host-beta")]
    #[async_trait]
    impl RebornUserIdentityBindingStore for FailingBindingStore {
        async fn bind_user_identity(
            &self,
            _binding: RebornUserIdentityBinding,
        ) -> Result<(), RebornUserIdentityBindingError> {
            Err(RebornUserIdentityBindingError::ProviderIdentityAlreadyBound)
        }

        async fn bind_user_identity_for_epoch(
            &self,
            _binding: RebornUserIdentityBinding,
            _epoch: SlackConnectionEpoch,
        ) -> Result<
            crate::slack::slack_personal_binding::SlackUserIdentityBindingRollback,
            RebornUserIdentityBindingError,
        > {
            Err(RebornUserIdentityBindingError::ProviderIdentityAlreadyBound)
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[async_trait]
    impl crate::slack::slack_personal_binding::RebornUserIdentityBindingDeleteStore
        for RecordingBindingStore
    {
        async fn user_identity_bindings_for_user(
            &self,
            provider: &str,
            user_id: &ironclaw_host_api::UserId,
            provider_user_id_prefix: Option<&str>,
        ) -> Result<Vec<SlackUserIdentityCleanupBinding>, RebornUserIdentityBindingError> {
            let bindings = self.bindings.lock().expect("binding store lock");
            let binding_epochs = self.binding_epochs.lock().expect("binding epoch lock");
            Ok(bindings
                .iter()
                .zip(binding_epochs.iter())
                .filter(|(binding, _)| {
                    binding.provider.as_str() == provider
                        && binding.user_id == *user_id
                        && provider_user_id_prefix.is_none_or(|prefix| {
                            binding.provider_user_id.as_str().starts_with(prefix)
                        })
                })
                .map(|(binding, epoch)| {
                    SlackUserIdentityCleanupBinding::new(binding.clone(), *epoch)
                })
                .collect())
        }

        async fn user_identity_bindings_for_user_at_epoch(
            &self,
            provider: &str,
            user_id: &ironclaw_host_api::UserId,
            provider_user_id_prefix: Option<&str>,
            expected_epoch: Option<SlackConnectionEpoch>,
        ) -> Result<Vec<SlackUserIdentityCleanupBinding>, RebornUserIdentityBindingError> {
            let bindings = self.bindings.lock().expect("binding store lock");
            let binding_epochs = self.binding_epochs.lock().expect("binding epoch lock");
            Ok(bindings
                .iter()
                .zip(binding_epochs.iter())
                .filter(|(binding, epoch)| {
                    binding.provider.as_str() == provider
                        && binding.user_id == *user_id
                        && provider_user_id_prefix.is_none_or(|prefix| {
                            binding.provider_user_id.as_str().starts_with(prefix)
                        })
                        && expected_epoch.is_none_or(|expected| **epoch == Some(expected))
                })
                .map(|(binding, epoch)| {
                    SlackUserIdentityCleanupBinding::new(binding.clone(), *epoch)
                })
                .collect())
        }

        async fn delete_user_identity_bindings_for_user_at_epoch(
            &self,
            provider: &str,
            user_id: &ironclaw_host_api::UserId,
            provider_user_id_prefix: Option<&str>,
            expected_epoch: Option<SlackConnectionEpoch>,
        ) -> Result<Vec<SlackUserIdentityCleanupBinding>, RebornUserIdentityBindingError> {
            let mut fail_delete_once = self.fail_delete_once.lock().expect("delete failure lock");
            if *fail_delete_once {
                *fail_delete_once = false;
                return Err(RebornUserIdentityBindingError::Backend(
                    "scripted identity rollback failure".to_string(),
                ));
            }
            drop(fail_delete_once);
            let mut bindings = self.bindings.lock().expect("binding store lock");
            let mut binding_epochs = self.binding_epochs.lock().expect("binding epoch lock");
            let mut deleted = Vec::new();
            for index in (0..bindings.len()).rev() {
                let binding = &bindings[index];
                let binding_epoch = binding_epochs[index];
                let should_delete = binding.provider.as_str() == provider
                    && binding.user_id == *user_id
                    && provider_user_id_prefix
                        .is_none_or(|prefix| binding.provider_user_id.as_str().starts_with(prefix))
                    && expected_epoch.is_none_or(|expected| binding_epoch == Some(expected));
                if should_delete {
                    deleted.push(SlackUserIdentityCleanupBinding::new(
                        bindings.remove(index),
                        binding_epochs.remove(index),
                    ));
                }
            }
            deleted.reverse();
            Ok(deleted)
        }
    }

    /// Delegates every flow operation to the in-memory fake but fails
    /// `complete_oauth_callback`, modeling a completion failure (flow-store
    /// IO, CAS mismatch) that lands after the identity hook already bound.
    #[cfg(feature = "slack-v2-host-beta")]
    struct FailingCompletionFlowManager {
        inner: Arc<InMemoryAuthProductServices>,
        fail_create: bool,
    }

    /// Delegates every flow operation to the in-memory fake but fails the
    /// first continuation marker write after the continuation side effect has
    /// already succeeded.
    #[cfg(feature = "slack-v2-host-beta")]
    struct FailingOnceContinuationMarkerFlowManager {
        inner: Arc<InMemoryAuthProductServices>,
        fail_marker_once: AtomicBool,
        marker_calls: AtomicUsize,
    }

    #[cfg(feature = "slack-v2-host-beta")]
    impl FailingOnceContinuationMarkerFlowManager {
        fn new(inner: Arc<InMemoryAuthProductServices>) -> Self {
            Self {
                inner,
                fail_marker_once: AtomicBool::new(true),
                marker_calls: AtomicUsize::new(0),
            }
        }

        fn marker_calls(&self) -> usize {
            self.marker_calls.load(Ordering::SeqCst)
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[async_trait]
    impl ironclaw_auth::AuthFlowManager for FailingCompletionFlowManager {
        async fn create_flow(
            &self,
            request: ironclaw_auth::NewAuthFlow,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            if self.fail_create {
                return Err(AuthProductError::CredentialMissing);
            }
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

        async fn claim_continuation_dispatch(
            &self,
            scope: &AuthProductScope,
            input: ironclaw_auth::AuthContinuationDispatchClaimInput,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            self.inner.claim_continuation_dispatch(scope, input).await
        }

        async fn settle_continuation_dispatch(
            &self,
            scope: &AuthProductScope,
            input: ironclaw_auth::AuthContinuationDispatchSettlementInput,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            self.inner.settle_continuation_dispatch(scope, input).await
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

    #[cfg(feature = "slack-v2-host-beta")]
    #[async_trait]
    impl ironclaw_auth::AuthFlowManager for FailingOnceContinuationMarkerFlowManager {
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
            scope: &AuthProductScope,
            input: ironclaw_auth::OAuthCallbackInput,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            self.inner.complete_oauth_callback(scope, input).await
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

        async fn claim_continuation_dispatch(
            &self,
            scope: &AuthProductScope,
            input: ironclaw_auth::AuthContinuationDispatchClaimInput,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            self.inner.claim_continuation_dispatch(scope, input).await
        }

        async fn settle_continuation_dispatch(
            &self,
            scope: &AuthProductScope,
            input: ironclaw_auth::AuthContinuationDispatchSettlementInput,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            self.marker_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_marker_once.swap(false, Ordering::SeqCst) {
                return Err(AuthProductError::BackendUnavailable);
            }
            self.inner.settle_continuation_dispatch(scope, input).await
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
            fail_create: false,
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
        .with_test_installed_extension_lookup()
        .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
        .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
            binding_service,
            Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                SlackPersonalConnectionScope { installation_id },
            ))),
            binding_store.clone(),
            Arc::new(TestSlackLifecycleStore::default()),
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

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_personal_oauth_callback_compensates_when_lifecycle_activation_fails() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let provider_identity = OAuthProviderIdentity::new(
            "U123",
            Some("T123".to_string()),
            Some("E123".to_string()),
            Some("A123".to_string()),
        )
        .expect("provider identity");
        let provider_client = Arc::new(SlackIdentityProviderClient::new(provider_identity));
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(
                shared.clone(),
                Arc::new(RejectingContinuationDispatcher),
            )
            .with_flow_record_source(shared.clone())
            .with_provider_client(provider_client.clone()),
        );
        let tenant_id = TenantId::new("tenant-alpha").expect("tenant");
        let user_id = UserId::new("user-alpha").expect("user");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation");
        let owner =
            SlackConnectionOwner::new(tenant_id.clone(), user_id.clone(), installation_id.clone());
        let lifecycle_store = Arc::new(TestSlackLifecycleStore::default());
        let binding_store = Arc::new(RecordingBindingStore::default());
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant_id.clone(),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            Arc::new(ActivatingBindingStore {
                inner: binding_store.clone(),
                lifecycle_store: lifecycle_store.clone(),
                owner: owner.clone(),
            }),
        ));
        let state = ProductAuthRouteState::new(product_auth, tenant_id.clone(), None, None)
            .with_test_installed_extension_lookup()
            .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
            .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
                binding_service,
                Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                    SlackPersonalConnectionScope {
                        installation_id: installation_id.clone(),
                    },
                ))),
                binding_store.clone(),
                lifecycle_store.clone(),
            ));
        let invocation_id = InvocationId::new();
        let Json(start_response) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(WebUiAuthenticatedCaller::new(
                tenant_id, user_id, None, None,
            )),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack".to_string(),
                scopes: vec!["search:read".to_string()],
                expires_at: Utc::now() + ChronoDuration::seconds(1),
                invocation_id: Some(invocation_id.to_string()),
            }),
        )
        .await
        .expect("start Slack OAuth");
        assert!(matches!(
            start_response.continuation,
            AuthContinuationRef::LifecycleActivation { .. }
        ));
        let state_value = Url::parse(start_response.authorization_url.as_str())
            .expect("authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("OAuth state");
        let callback_state = OAuthCallbackState::decode(
            OAuthCallbackStateKind::SLACK_PERSONAL,
            state_value.as_str(),
        )
        .expect("callback state");
        let unrelated_account = shared
            .create_account(NewCredentialAccount {
                scope: callback_state.scope().clone(),
                provider: AuthProviderId::new(SLACK_PERSONAL_PROVIDER_ID).expect("provider"),
                label: CredentialAccountLabel::new("unrelated slack credential")
                    .expect("account label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("unrelated-slack-access").expect("secret")),
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .expect("unrelated same-provider account");
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let uri = format!(
            "{SLACK_PERSONAL_OAUTH_CALLBACK_PATH}?state={encoded_state}&code=slack-auth-code"
        )
        .parse::<Uri>()
        .expect("callback uri");

        let error = slack_personal_oauth_callback_handler(
            State(state.clone()),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            HeaderMap::new(),
        )
        .await
        .expect_err("lifecycle activation failure must surface");

        assert_eq!(error.body.code, AuthErrorCode::BackendUnavailable);
        assert_eq!(provider_client.calls(), 1, "credential exchange completed");
        let completed_flow = shared
            .flow_records_snapshot()
            .into_iter()
            .find(|flow| flow.id == start_response.flow_id)
            .expect("completed OAuth flow remains durable");
        assert_eq!(completed_flow.status, AuthFlowStatus::Failed);
        assert!(completed_flow.credential_account_id.is_some());
        assert!(
            binding_store.bindings().is_empty(),
            "failed lifecycle activation must roll back the Slack identity"
        );
        assert_eq!(
            lifecycle_store
                .connection_state(&owner)
                .await
                .expect("rolled-back connection state"),
            Some((
                SlackConnectionEpoch::new(start_response.flow_id),
                SlackConnectionState::Disconnected,
            )),
            "failed lifecycle activation must abandon the Slack connection epoch"
        );
        let accounts = shared
            .accounts_for_owner(&callback_state.scope().to_credential_owner())
            .await
            .expect("credential account after lifecycle compensation");
        assert_eq!(accounts.len(), 2);
        let failed_account = accounts
            .iter()
            .find(|account| Some(account.id) == completed_flow.credential_account_id)
            .expect("failed callback account");
        assert_eq!(failed_account.status, CredentialAccountStatus::Revoked);
        assert!(failed_account.access_secret.is_none());
        assert!(failed_account.refresh_secret.is_none());
        let unrelated_account = accounts
            .iter()
            .find(|account| account.id == unrelated_account.id)
            .expect("unrelated account remains");
        assert_eq!(
            unrelated_account.status,
            CredentialAccountStatus::Configured,
            "activation compensation must not revoke another account for the same provider"
        );
        assert!(unrelated_account.access_secret.is_some());

        // Recreate the exact stale terminal state a restart can expose after
        // the callback wrapper failed between durable Failed settlement and
        // provider cleanup. The first status attempt has an injected binding
        // delete failure; it must remain retryable, and a later poll must still
        // converge even after the interactive OAuth flow has expired.
        let connection_epoch = SlackConnectionEpoch::new(start_response.flow_id);
        lifecycle_store
            .begin_connection(
                &owner,
                connection_epoch,
                Utc::now() + ChronoDuration::minutes(5),
            )
            .await
            .expect("recreate stale lifecycle owner");
        binding_store
            .bind_user_identity_for_epoch(
                RebornUserIdentityBinding {
                    provider: RebornIdentityProviderId::new(
                        crate::slack::slack_actor_identity::SLACK_IDENTITY_PROVIDER,
                    )
                    .expect("identity provider"),
                    provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123")
                        .expect("provider user id"),
                    user_id: callback_state.scope().resource.user_id.clone(),
                },
                connection_epoch,
            )
            .await
            .expect("recreate stale identity binding");
        binding_store.fail_next_delete();
        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
        let caller = WebUiAuthenticatedCaller::new(
            callback_state.scope().resource.tenant_id.clone(),
            callback_state.scope().resource.user_id.clone(),
            None,
            None,
        );
        let status_query = OAuthFlowStatusQuery {
            invocation_id: Some(callback_state.scope().resource.invocation_id.to_string()),
        };
        let Json(observed_status) = oauth_flow_status_handler(
            State(state.clone()),
            Extension(caller.clone()),
            Path(start_response.flow_id.to_string()),
            axum::extract::Query(status_query),
        )
        .await
        .expect("observational status read");
        assert_eq!(observed_status.status, AuthFlowStatus::Failed);
        assert_eq!(
            binding_store.bindings().len(),
            1,
            "GET status must not run provider-owned cleanup"
        );
        assert_eq!(
            lifecycle_store
                .connection_state(&owner)
                .await
                .expect("connection state after observational read"),
            Some((connection_epoch, SlackConnectionState::Connecting)),
            "GET status must not mutate lifecycle state"
        );
        let status_query = OAuthFlowStatusQuery {
            invocation_id: Some(callback_state.scope().resource.invocation_id.to_string()),
        };
        let first_status = oauth_flow_reconcile_handler(
            State(state.clone()),
            Extension(caller.clone()),
            Path(start_response.flow_id.to_string()),
            axum::extract::Query(status_query),
        )
        .await
        .expect_err("failed binding cleanup keeps terminal status retryable");
        assert_eq!(first_status.body.code, AuthErrorCode::BackendUnavailable);
        assert_eq!(
            lifecycle_store
                .connection_state(&owner)
                .await
                .expect("fenced connection state"),
            Some((connection_epoch, SlackConnectionState::Disconnecting)),
            "the failed epoch must be fenced before fallible identity deletion"
        );
        binding_store.fail_next_delete();
        let second_status = oauth_flow_reconcile_handler(
            State(state.clone()),
            Extension(caller.clone()),
            Path(start_response.flow_id.to_string()),
            axum::extract::Query(OAuthFlowStatusQuery {
                invocation_id: Some(callback_state.scope().resource.invocation_id.to_string()),
            }),
        )
        .await
        .expect_err("a second identity failure remains retryable and fenced");
        assert_eq!(second_status.body.code, AuthErrorCode::BackendUnavailable);
        assert_eq!(
            lifecycle_store
                .connection_state(&owner)
                .await
                .expect("connection state after second cleanup failure"),
            Some((connection_epoch, SlackConnectionState::Disconnecting))
        );
        let Json(status_response) = oauth_flow_reconcile_handler(
            State(state),
            Extension(caller),
            Path(start_response.flow_id.to_string()),
            axum::extract::Query(OAuthFlowStatusQuery {
                invocation_id: Some(callback_state.scope().resource.invocation_id.to_string()),
            }),
        )
        .await
        .expect("expired failed flow cleanup converges on retry");
        assert_eq!(status_response.status, AuthFlowStatus::Failed);
        assert!(binding_store.bindings().is_empty());
        assert_eq!(
            lifecycle_store
                .connection_state(&owner)
                .await
                .expect("terminal connection state"),
            Some((connection_epoch, SlackConnectionState::Disconnected))
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_personal_oauth_marker_failure_preserves_connection_and_retries_ack() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let provider_identity = OAuthProviderIdentity::new(
            "U123",
            Some("T123".to_string()),
            Some("E123".to_string()),
            Some("A123".to_string()),
        )
        .expect("provider identity");
        let provider_client = Arc::new(SlackIdentityProviderClient::new(provider_identity));
        let flow_manager = Arc::new(FailingOnceContinuationMarkerFlowManager::new(
            shared.clone(),
        ));
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let product_auth = Arc::new(RebornProductAuthServices::new(
            flow_manager.clone(),
            shared.clone(),
            shared.clone(),
            shared.clone(),
            provider_client.clone(),
            shared.clone(),
            dispatcher.clone(),
        ));
        let tenant_id = TenantId::new("tenant-alpha").expect("tenant");
        let user_id = UserId::new("user-alpha").expect("user");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation");
        let owner =
            SlackConnectionOwner::new(tenant_id.clone(), user_id.clone(), installation_id.clone());
        let lifecycle_store = Arc::new(TestSlackLifecycleStore::default());
        let binding_store = Arc::new(RecordingBindingStore::default());
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant_id.clone(),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            Arc::new(ActivatingBindingStore {
                inner: binding_store.clone(),
                lifecycle_store: lifecycle_store.clone(),
                owner: owner.clone(),
            }),
        ));
        let state = ProductAuthRouteState::new(product_auth, tenant_id.clone(), None, None)
            .with_test_installed_extension_lookup()
            .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
            .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
                binding_service,
                Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                    SlackPersonalConnectionScope {
                        installation_id: installation_id.clone(),
                    },
                ))),
                binding_store.clone(),
                lifecycle_store.clone(),
            ));
        let Json(start_response) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(WebUiAuthenticatedCaller::new(
                tenant_id, user_id, None, None,
            )),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack".to_string(),
                scopes: vec!["search:read".to_string()],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(InvocationId::new().to_string()),
            }),
        )
        .await
        .expect("start Slack OAuth");
        let state_value = Url::parse(start_response.authorization_url.as_str())
            .expect("authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("OAuth state");
        let callback_state = OAuthCallbackState::decode(
            OAuthCallbackStateKind::SLACK_PERSONAL,
            state_value.as_str(),
        )
        .expect("callback state");
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let uri = format!(
            "{SLACK_PERSONAL_OAUTH_CALLBACK_PATH}?state={encoded_state}&code=slack-auth-code"
        )
        .parse::<Uri>()
        .expect("callback uri");

        let error = slack_personal_oauth_callback_handler(
            State(state.clone()),
            RawQuery(uri.query().map(str::to_string)),
            uri.clone(),
            HeaderMap::new(),
        )
        .await
        .expect_err("first continuation marker write fails");

        assert_eq!(error.body.code, AuthErrorCode::BackendUnavailable);
        assert_eq!(provider_client.calls(), 1, "OAuth exchange ran once");
        assert_eq!(dispatcher.events().len(), 1, "activation dispatched once");
        assert_eq!(
            flow_manager.marker_calls(),
            2,
            "failed acknowledgement releases the durable claim"
        );
        assert_eq!(
            state
                .product_auth
                .reconcile_oauth_flow(callback_state.scope(), start_response.flow_id)
                .await
                .expect("status after acknowledgement failure"),
            AuthFlowStatus::Completed,
            "status polling redrives and acknowledges the released lifecycle claim"
        );
        assert_eq!(binding_store.bindings().len(), 1);
        assert_eq!(
            lifecycle_store
                .connection_state(&owner)
                .await
                .expect("active connection state"),
            Some((
                SlackConnectionEpoch::new(start_response.flow_id),
                SlackConnectionState::Active,
            )),
            "an acknowledgement failure must not tear down a successful activation"
        );
        let accounts = shared
            .accounts_for_owner(&callback_state.scope().to_credential_owner())
            .await
            .expect("credential account after marker failure");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].status, CredentialAccountStatus::Configured);
        assert!(accounts[0].access_secret.is_some());

        slack_personal_oauth_callback_handler(
            State(state),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            HeaderMap::new(),
        )
        .await
        .expect("callback retry re-dispatches idempotently and persists the marker");

        assert_eq!(provider_client.calls(), 1, "retry must not exchange twice");
        assert_eq!(dispatcher.events().len(), 2, "delivery is at least once");
        assert_eq!(
            flow_manager.marker_calls(),
            3,
            "status polling, not provider exchange, persists the retry marker"
        );
        let completed = shared
            .flow_records_snapshot()
            .into_iter()
            .find(|flow| flow.id == start_response.flow_id)
            .expect("completed flow");
        assert_eq!(completed.status, AuthFlowStatus::Completed);
        assert!(completed.continuation_emitted_at.is_some());
        assert_eq!(binding_store.bindings().len(), 1);
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_personal_oauth_activation_failure_after_reconfigure_fully_disconnects_owner() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let provider_identity = OAuthProviderIdentity::new(
            "U123",
            Some("T123".to_string()),
            Some("E123".to_string()),
            Some("A123".to_string()),
        )
        .expect("provider identity");
        let provider_client = Arc::new(SlackIdentityProviderClient::new(provider_identity));
        let dispatcher = Arc::new(RejectingSecondContinuationDispatcher::default());
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), dispatcher)
                .with_flow_record_source(shared.clone())
                .with_provider_client(provider_client),
        );
        let tenant_id = TenantId::new("tenant-alpha").expect("tenant");
        let user_id = UserId::new("user-alpha").expect("user");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation");
        let owner =
            SlackConnectionOwner::new(tenant_id.clone(), user_id.clone(), installation_id.clone());
        let lifecycle_store = Arc::new(TestSlackLifecycleStore::default());
        let binding_store = Arc::new(RecordingBindingStore::default());
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant_id.clone(),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            Arc::new(ActivatingBindingStore {
                inner: binding_store.clone(),
                lifecycle_store: lifecycle_store.clone(),
                owner: owner.clone(),
            }),
        ));
        let state = ProductAuthRouteState::new(product_auth, tenant_id.clone(), None, None)
            .with_test_installed_extension_lookup()
            .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
            .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
                binding_service,
                Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                    SlackPersonalConnectionScope {
                        installation_id: installation_id.clone(),
                    },
                ))),
                binding_store.clone(),
                lifecycle_store.clone(),
            ));
        let caller = WebUiAuthenticatedCaller::new(tenant_id.clone(), user_id.clone(), None, None);
        let callback_uri = |authorization_url: &OAuthAuthorizationUrl| {
            let state_value = Url::parse(authorization_url.as_str())
                .expect("authorization url")
                .query_pairs()
                .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
                .expect("OAuth state");
            let encoded_state =
                url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
            let uri = format!(
                "{SLACK_PERSONAL_OAUTH_CALLBACK_PATH}?state={encoded_state}&code=slack-auth-code"
            )
            .parse::<Uri>()
            .expect("callback uri");
            (state_value, uri)
        };

        let Json(first_start) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(caller.clone()),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack".to_string(),
                scopes: vec!["search:read".to_string()],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(InvocationId::new().to_string()),
            }),
        )
        .await
        .expect("start initial Slack OAuth");
        let (first_state_value, first_uri) = callback_uri(&first_start.authorization_url);
        let first_callback_state = OAuthCallbackState::decode(
            OAuthCallbackStateKind::SLACK_PERSONAL,
            first_state_value.as_str(),
        )
        .expect("first callback state");
        slack_personal_oauth_callback_handler(
            State(state.clone()),
            RawQuery(first_uri.query().map(str::to_string)),
            first_uri,
            HeaderMap::new(),
        )
        .await
        .expect("initial Slack activation succeeds");
        let initial_accounts = shared
            .accounts_for_owner(&first_callback_state.scope().to_credential_owner())
            .await
            .expect("initial credential account");
        assert_eq!(initial_accounts.len(), 1);
        assert_eq!(
            initial_accounts[0].status,
            CredentialAccountStatus::Configured
        );
        let existing_account_id = initial_accounts[0].id;

        let Json(reconfigure) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(caller),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack replacement".to_string(),
                scopes: vec!["search:read".to_string()],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(InvocationId::new().to_string()),
            }),
        )
        .await
        .expect("start Slack reconfigure");
        let (_, reconfigure_uri) = callback_uri(&reconfigure.authorization_url);

        slack_personal_oauth_callback_handler(
            State(state),
            RawQuery(reconfigure_uri.query().map(str::to_string)),
            reconfigure_uri,
            HeaderMap::new(),
        )
        .await
        .expect_err("second lifecycle activation is rejected");

        let accounts = shared
            .accounts_for_owner(&first_callback_state.scope().to_credential_owner())
            .await
            .expect("credential account after failed reconfigure");
        assert_eq!(
            accounts.len(),
            1,
            "reconfigure updates the existing account"
        );
        assert_eq!(accounts[0].id, existing_account_id);
        assert_eq!(accounts[0].status, CredentialAccountStatus::Revoked);
        assert!(accounts[0].access_secret.is_none());
        assert!(accounts[0].refresh_secret.is_none());
        assert!(
            binding_store.bindings().is_empty(),
            "the previous identity must not be restored after its credential was overwritten"
        );
        assert_eq!(
            lifecycle_store
                .connection_state(&owner)
                .await
                .expect("disconnected lifecycle"),
            Some((
                SlackConnectionEpoch::new(reconfigure.flow_id),
                SlackConnectionState::Disconnected,
            )),
            "failed reconfigure activation must fully disconnect the Slack owner"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_personal_oauth_failed_reconfigure_restores_previous_active_binding() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let provider_identity = OAuthProviderIdentity::new(
            "U123",
            Some("T123".to_string()),
            Some("E123".to_string()),
            Some("A123".to_string()),
        )
        .expect("provider identity");
        let provider_client = Arc::new(SlackIdentityProviderClient::new(provider_identity));
        let product_auth = Arc::new(RebornProductAuthServices::new(
            Arc::new(FailingCompletionFlowManager {
                inner: shared.clone(),
                fail_create: false,
            }),
            shared.clone(),
            shared.clone(),
            shared.clone(),
            provider_client,
            shared,
            Arc::new(RecordingDispatcher::default()),
        ));
        let tenant_id = TenantId::new("tenant-alpha").expect("tenant");
        let user_id = UserId::new("user-alpha").expect("user");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation");
        let owner =
            SlackConnectionOwner::new(tenant_id.clone(), user_id.clone(), installation_id.clone());
        let lifecycle_store = Arc::new(TestSlackLifecycleStore::default());
        let binding_store = Arc::new(RecordingBindingStore::default());
        let activating_store = Arc::new(ActivatingBindingStore {
            inner: binding_store.clone(),
            lifecycle_store: lifecycle_store.clone(),
            owner: owner.clone(),
        });
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant_id.clone(),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            activating_store,
        ));
        let active_epoch = SlackConnectionEpoch::new(AuthFlowId::new());
        lifecycle_store
            .begin_connection(
                &owner,
                active_epoch,
                Utc::now() + ChronoDuration::minutes(5),
            )
            .await
            .expect("initial connection begins");
        binding_service
            .bind_personal_user_for_epoch(
                crate::slack::slack_personal_binding::SlackPersonalBindingPrincipal {
                    tenant_id: tenant_id.clone(),
                    user_id: user_id.clone(),
                },
                crate::slack::slack_personal_binding::SlackPersonalUserBindingRequest {
                    installation_id: installation_id.clone(),
                    slack_user_id: crate::slack::slack_serve::SlackUserId::new("U123"),
                    team_id: crate::slack::slack_serve::SlackTeamId::new("T123"),
                    enterprise_id: Some(crate::slack::slack_serve::SlackEnterpriseId::new("E123")),
                    api_app_id: crate::slack::slack_serve::SlackApiAppId::new("A123"),
                },
                active_epoch,
            )
            .await
            .expect("initial binding activates");

        let state = ProductAuthRouteState::new(product_auth, tenant_id.clone(), None, None)
            .with_test_installed_extension_lookup()
            .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
            .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
                binding_service,
                Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                    SlackPersonalConnectionScope {
                        installation_id: installation_id.clone(),
                    },
                ))),
                binding_store.clone(),
                lifecycle_store.clone(),
            ));
        let Json(start_response) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(WebUiAuthenticatedCaller::new(
                tenant_id, user_id, None, None,
            )),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack".to_string(),
                scopes: vec!["search:read".to_string()],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(InvocationId::new().to_string()),
            }),
        )
        .await
        .expect("start reconfigure flow");
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
        .expect_err("injected completion failure must surface");

        assert_eq!(
            lifecycle_store
                .connection_state(&owner)
                .await
                .expect("restored lifecycle"),
            Some((active_epoch, SlackConnectionState::Active))
        );
        assert_eq!(
            binding_store.bindings().len(),
            1,
            "failed replacement must leave exactly the previous active identity"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_personal_oauth_failed_identity_rollback_allows_disconnect_then_reconnect() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let provider_identity = OAuthProviderIdentity::new(
            "U123",
            Some("T123".to_string()),
            Some("E123".to_string()),
            Some("A123".to_string()),
        )
        .expect("provider identity");
        let provider_client = Arc::new(SlackIdentityProviderClient::new(provider_identity));
        let product_auth = Arc::new(RebornProductAuthServices::new(
            Arc::new(FailingCompletionFlowManager {
                inner: shared.clone(),
                fail_create: false,
            }),
            shared.clone(),
            shared.clone(),
            shared.clone(),
            provider_client,
            shared,
            Arc::new(RecordingDispatcher::default()),
        ));
        let tenant_id = TenantId::new("tenant-alpha").expect("tenant");
        let user_id = UserId::new("user-alpha").expect("user");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation");
        let binding_store = Arc::new(RecordingBindingStore::default());
        binding_store.fail_next_delete();
        let lifecycle_store = Arc::new(TestSlackLifecycleStore::default());
        let owner =
            SlackConnectionOwner::new(tenant_id.clone(), user_id.clone(), installation_id.clone());
        let binding_service = Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant_id.clone(),
                installation_id: installation_id.clone(),
                selector: SlackInstallationSelector::app_team("A123", "T123"),
            }],
            Arc::new(ActivatingBindingStore {
                inner: binding_store.clone(),
                lifecycle_store: lifecycle_store.clone(),
                owner: owner.clone(),
            }),
        ));
        let state = ProductAuthRouteState::new(product_auth, tenant_id.clone(), None, None)
            .with_test_installed_extension_lookup()
            .with_slack_personal_oauth(slack_personal_oauth_test_slot().await)
            .with_slack_personal_oauth_binding(SlackPersonalOAuthBindingConfig::new(
                binding_service,
                Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                    SlackPersonalConnectionScope {
                        installation_id: installation_id.clone(),
                    },
                ))),
                binding_store.clone(),
                lifecycle_store.clone(),
            ));
        let caller = WebUiAuthenticatedCaller::new(tenant_id.clone(), user_id.clone(), None, None);
        let invocation_id = InvocationId::new();
        let Json(start_response) = extension_oauth_start_handler(
            State(state.clone()),
            Extension(caller.clone()),
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
        .expect("start Slack OAuth");
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
            State(state.clone()),
            RawQuery(uri.query().map(str::to_string)), // safety: URI query parsing, not a database query.
            uri,
            HeaderMap::new(),
        )
        .await
        .expect_err("flow completion failure surfaces");
        assert_eq!(
            binding_store.bindings().len(),
            1,
            "the scripted rollback failure leaves the identity for explicit cleanup"
        );

        let connection_epoch = SlackConnectionEpoch::new(start_response.flow_id);
        assert_eq!(
            lifecycle_store
                .connection_state(&owner)
                .await
                .expect("failed connection state"),
            Some((connection_epoch, SlackConnectionState::Disconnected)),
            "a failed identity rollback must never leave ingress active"
        );
        let fence = lifecycle_store
            .begin_disconnect(&owner)
            .await
            .expect("disconnect begins");
        binding_store
            .delete_user_identity_bindings_for_user_at_epoch(
                crate::slack::slack_actor_identity::SLACK_IDENTITY_PROVIDER,
                &user_id,
                Some("install-alpha:"),
                fence.cleanup_selector().epoch(),
            )
            .await
            .expect("disconnect retries the failed identity cleanup");
        lifecycle_store
            .complete_disconnect(&owner, fence.fence_epoch())
            .await
            .expect("disconnect completes");
        assert!(binding_store.bindings().is_empty());

        let _ = extension_oauth_start_handler(
            State(state),
            Extension(caller),
            Path("slack".to_string()),
            Json(ExtensionOAuthStartRequest {
                provider: SLACK_PERSONAL_PROVIDER_ID.to_string(),
                account_label: "personal slack retry".to_string(),
                scopes: vec!["search:read".to_string()],
                expires_at: Utc::now() + ChronoDuration::minutes(5),
                invocation_id: Some(InvocationId::new().to_string()),
            }),
        )
        .await
        .expect("a clean reconnect may start after disconnect retry");
    }
}
