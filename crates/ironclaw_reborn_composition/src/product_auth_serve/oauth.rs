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

pub(super) async fn google_oauth_start_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<GoogleOAuthStartRequest>,
) -> Result<Json<GoogleOAuthStartResponse>, ProductAuthRouteFailure> {
    start_google_oauth_flow(state, caller, request, None, false).await
}

pub(super) async fn extension_oauth_start_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(package_id): Path<String>,
    Json(request): Json<ExtensionOAuthStartRequest>,
) -> Result<Json<GoogleOAuthStartResponse>, ProductAuthRouteFailure> {
    if request.provider != GOOGLE_PROVIDER_ID {
        return Err(ProductAuthRouteFailure::invalid_request());
    }
    let requester_extension =
        ExtensionId::new(package_id).map_err(|_| ProductAuthRouteFailure::invalid_request())?;
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

async fn start_google_oauth_flow(
    state: ProductAuthRouteState,
    caller: WebUiAuthenticatedCaller,
    request: GoogleOAuthStartRequest,
    requester_extension: Option<ExtensionId>,
    require_invocation_id: bool,
) -> Result<Json<GoogleOAuthStartResponse>, ProductAuthRouteFailure> {
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
        requested_scopes.clone(),
        requester_extension.as_ref(),
    )
    .await?;
    let opaque_state = GoogleOAuthCallbackState::new(
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

    Ok(Json(GoogleOAuthStartResponse {
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

    let flow_provider = if is_authorized_callback_candidate(&query) {
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
    let outcome =
        callback_outcome_from_query(&state, flow_id, &scope, flow_provider.as_ref(), &query)
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

    Ok(Json(response))
}

pub(super) async fn google_oauth_callback_handler(
    State(state): State<ProductAuthRouteState>,
    RawQuery(raw_query): RawQuery,
    uri: Uri,
) -> Result<Json<RebornOAuthCallbackResponse>, ProductAuthRouteFailure> {
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
    let callback_state = GoogleOAuthCallbackState::decode(state_value.as_str())
        .map_err(ProductAuthRouteFailure::from)?;
    let flow_id = callback_state.flow_id();
    let callback_scope = callback_state.scope();

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
        return response.map(Json);
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
    let pkce_verifier = match state.pkce_verifier_for_callback(flow_id) {
        Ok(pkce_verifier) => pkce_verifier,
        Err(_) => match run_with_backend_timeout(state.product_auth.oauth_pkce_verifier_for_flow(
            callback_scope,
            &provider,
            flow_id,
        ))
        .await?
        {
            Some(pkce_verifier) => pkce_verifier,
            None => {
                state.remove_pkce_verifier(flow_id);
                return Err(ProductAuthRouteFailure::unknown_or_expired_flow());
            }
        },
    };
    let callback_scopes = match parse_google_callback_scopes(query.scopes.as_deref()) {
        Ok(callback_scopes) => {
            callback_scopes.unwrap_or_else(|| callback_state.requested_scopes().to_vec())
        }
        Err(error) => {
            state.remove_pkce_verifier(flow_id);
            return Err(ProductAuthRouteFailure::from(error));
        }
    };
    if callback_scopes.is_empty() {
        state.remove_pkce_verifier(flow_id);
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    let authorization_code_hash = authorization_code_hash(code.expose_secret())?;
    let pkce_verifier_hash = pkce_verifier_hash(pkce_verifier.expose_secret())?;

    let response = match run_with_backend_timeout(
        state
            .product_auth
            .handle_oauth_callback(RebornOAuthCallbackRequest {
                scope: callback_scope.clone(),
                flow_id,
                opaque_state_hash: state_hash.clone(),
                outcome: RebornOAuthCallbackOutcome::Authorized {
                    provider_request: OAuthProviderCallbackRequest {
                        provider: AuthProviderId::new(GOOGLE_PROVIDER_ID)
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
            }),
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

    Ok(Json(response))
}

pub(super) async fn callback_outcome_from_query(
    state: &ProductAuthRouteState,
    flow_id: AuthFlowId,
    scope: &AuthProductScope,
    flow_provider: Option<&AuthProviderId>,
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
    let provider = AuthProviderId::new(provider.to_string())
        .map_err(|_| ProductAuthRouteFailure::malformed_callback())?;
    if flow_provider.is_some_and(|known_provider| known_provider != &provider) {
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    let account_label = required_callback_value(query.account_label.as_deref())?;
    let code = query
        .code
        .as_ref()
        .ok_or_else(ProductAuthRouteFailure::malformed_callback)?;
    let pkce_verifier = match state.pkce_verifier_for_callback(flow_id) {
        Ok(verifier) => verifier,
        Err(cache_error) => state
            .product_auth
            .oauth_pkce_verifier_for_flow(scope, flow_provider.unwrap_or(&provider), flow_id)
            .await?
            .ok_or(cache_error)?,
    };
    let scopes = parse_provider_scopes(query.scopes.as_deref())?;
    let authorization_code_hash = authorization_code_hash(code.expose_secret())?;
    let pkce_verifier_hash = pkce_verifier_hash(pkce_verifier.expose_secret())?;

    Ok(RebornOAuthCallbackOutcome::Authorized {
        provider_request: OAuthProviderCallbackRequest {
            provider,
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
