//! OAuth start and callback handlers.

use super::*;

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
    state.ensure_pkce_verifier_capacity()?;

    let flow = run_with_backend_timeout(
        state
            .product_auth
            .start_setup_oauth_flow(RebornOAuthStartFlowRequest {
                scope: scope.clone(),
                provider: provider.clone(),
                authorization_url: OAuthAuthorizationUrl::new(authorization_endpoint.to_string())
                    .map_err(ProductAuthRouteFailure::from)?,
                opaque_state_hash,
                pkce_verifier_hash,
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

pub(super) async fn oauth_callback_handler(
    State(state): State<ProductAuthRouteState>,
    Path(flow_id): Path<String>,
    RawQuery(raw_query): RawQuery,
    uri: Uri,
) -> Result<Json<RebornOAuthCallbackResponse>, ProductAuthRouteFailure> {
    validate_callback_raw_query(raw_query.as_deref())?;
    let query = axum::extract::Query::<OAuthCallbackQuery>::try_from_uri(&uri)
        .map_err(|_| ProductAuthRouteFailure::malformed_callback())?
        .0;
    validate_callback_query_fields(&query)?;

    let flow_id = AuthFlowId::from_uuid(
        Uuid::parse_str(&flow_id).map_err(|_| ProductAuthRouteFailure::malformed_callback())?,
    );
    let scope = scope_from_callback_query(&state, &query)?;
    let state_hash = opaque_state_hash(
        query
            .state
            .as_ref()
            .ok_or_else(ProductAuthRouteFailure::malformed_callback)?
            .as_str(),
    )?;

    if is_authorized_callback_candidate(&query) {
        run_with_backend_timeout(
            state
                .product_auth
                .ensure_oauth_callback_flow_known(&scope, flow_id),
        )
        .await?;
    }
    let outcome = callback_outcome_from_query(&state, flow_id, &scope, &query)?;

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

    Ok(Json(response))
}

pub(super) fn callback_outcome_from_query(
    state: &ProductAuthRouteState,
    flow_id: AuthFlowId,
    _scope: &AuthProductScope,
    query: &OAuthCallbackQuery,
) -> Result<RebornOAuthCallbackOutcome, ProductAuthRouteFailure> {
    if query
        .error
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return Ok(RebornOAuthCallbackOutcome::ProviderDenied);
    }

    let provider = required_callback_value(query.provider.as_deref())?;
    let account_label = required_callback_value(query.account_label.as_deref())?;
    let code = query
        .code
        .as_ref()
        .ok_or_else(ProductAuthRouteFailure::malformed_callback)?;
    let pkce_verifier = state.pkce_verifier_for_callback(flow_id)?;
    let scopes = parse_provider_scopes(query.scopes.as_deref())?;
    if scopes.is_empty() {
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    let authorization_code_hash = authorization_code_hash(code.expose_secret())?;
    let pkce_verifier_hash = pkce_verifier_hash(pkce_verifier.expose_secret())?;

    Ok(RebornOAuthCallbackOutcome::Authorized {
        provider_request: OAuthProviderCallbackRequest {
            provider: AuthProviderId::new(provider.to_string())
                .map_err(|_| ProductAuthRouteFailure::malformed_callback())?,
            account_label: CredentialAccountLabel::new(account_label.to_string())
                .map_err(|_| ProductAuthRouteFailure::malformed_callback())?,
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

pub(super) fn is_authorized_callback_candidate(query: &OAuthCallbackQuery) -> bool {
    query.error.as_deref().is_none_or(|value| value.is_empty())
        && query.provider.is_some()
        && query.account_label.is_some()
        && query.code.is_some()
}

pub(super) fn required_callback_value(
    value: Option<&str>,
) -> Result<&str, ProductAuthRouteFailure> {
    value.ok_or_else(ProductAuthRouteFailure::malformed_callback)
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
