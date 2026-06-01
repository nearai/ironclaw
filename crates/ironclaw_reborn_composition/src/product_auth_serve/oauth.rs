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

pub(super) async fn google_oauth_start_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<GoogleOAuthStartRequest>,
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
    let requested_scopes = parse_google_requested_scopes(&request.scopes)?;
    let scope = scope_from_authenticated_caller_parts(
        &caller,
        &ScopeFields {
            session_id: request.session_id,
            thread_id: request.thread_id,
            invocation_id: None,
        },
    )?;
    let opaque_state = generated_oauth_value("google-state")?;
    let pkce_verifier_value = generated_oauth_value("google-pkce")?;
    let opaque_state_hash = opaque_state_hash(opaque_state.as_str())?;
    let pkce_verifier_hash = pkce_verifier_hash(pkce_verifier_value.as_str())?;
    let pkce_verifier_secret = SecretString::from(pkce_verifier_value.as_str().to_string());
    let pkce_secret = PkceVerifierSecret::new(pkce_verifier_secret.clone())
        .map_err(ProductAuthRouteFailure::from)?;
    let pkce_challenge = pkce_s256_challenge(&pkce_secret);
    let authorization_url = build_google_authorization_url(
        config.client_id.as_str(),
        config.redirect_uri.as_str(),
        opaque_state.as_str(),
        &pkce_challenge,
        &requested_scopes,
        config.hosted_domain_hint.as_deref(),
    )
    .map_err(ProductAuthRouteFailure::from)?;

    let flow = run_with_backend_timeout(state.product_auth.start_setup_oauth_flow(
        RebornOAuthStartFlowRequest {
            scope: scope.clone(),
            provider: provider.clone(),
            authorization_url: authorization_url.clone(),
            opaque_state_hash: opaque_state_hash.clone(),
            pkce_verifier_hash,
            expires_at: request.expires_at,
        },
    ))
    .await?;
    state.store_pkce_verifier(flow.id, pkce_verifier_secret, flow.expires_at)?;
    if let Err(error) = state.store_pending_google_oauth(
        opaque_state_hash,
        PendingGoogleOAuthFlow {
            flow_id: flow.id,
            scope: scope.clone(),
            account_label,
            requested_scopes: requested_scopes.clone(),
            expires_at: flow.expires_at,
        },
    ) {
        state.remove_pkce_verifier(flow.id);
        return Err(error);
    }

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
    let pending = state.pending_google_oauth_flow(&state_hash)?;

    if query
        .error
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        let response = run_with_backend_timeout(state.product_auth.handle_oauth_callback(
            RebornOAuthCallbackRequest {
                scope: pending.scope.clone(),
                flow_id: pending.flow_id,
                opaque_state_hash: state_hash.clone(),
                outcome: RebornOAuthCallbackOutcome::ProviderDenied,
            },
        ))
        .await;
        state.remove_pkce_verifier(pending.flow_id);
        state.remove_pending_google_oauth(&state_hash);
        return response.map(Json);
    }

    if let Err(error) = run_with_backend_timeout(
        state
            .product_auth
            .ensure_oauth_callback_flow_known(&pending.scope, pending.flow_id),
    )
    .await
    {
        state.remove_pkce_verifier(pending.flow_id);
        state.remove_pending_google_oauth(&state_hash);
        return Err(error);
    }
    let code = query
        .code
        .as_ref()
        .ok_or_else(ProductAuthRouteFailure::malformed_callback)?;
    let pkce_verifier = state.pkce_verifier_for_callback(pending.flow_id)?;
    let callback_scopes = parse_google_callback_scopes(query.scopes.as_deref())?
        .unwrap_or_else(|| pending.requested_scopes.clone());
    if callback_scopes.is_empty() {
        state.remove_pkce_verifier(pending.flow_id);
        state.remove_pending_google_oauth(&state_hash);
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    let authorization_code_hash = authorization_code_hash(code.expose_secret())?;
    let pkce_verifier_hash = pkce_verifier_hash(pkce_verifier.expose_secret())?;

    let response = match run_with_backend_timeout(
        state
            .product_auth
            .handle_oauth_callback(RebornOAuthCallbackRequest {
                scope: pending.scope.clone(),
                flow_id: pending.flow_id,
                opaque_state_hash: state_hash.clone(),
                outcome: RebornOAuthCallbackOutcome::Authorized {
                    provider_request: OAuthProviderCallbackRequest {
                        provider: AuthProviderId::new(GOOGLE_PROVIDER_ID)
                            .map_err(|_| ProductAuthRouteFailure::malformed_callback())?,
                        account_label: pending.account_label,
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
            state.remove_pkce_verifier(pending.flow_id);
            state.remove_pending_google_oauth(&state_hash);
            response
        }
        Err(error) => {
            if should_forget_pkce_verifier(error.body.code) {
                state.remove_pkce_verifier(pending.flow_id);
                state.remove_pending_google_oauth(&state_hash);
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

fn generated_oauth_value(prefix: &str) -> Result<RawCallbackValue, ProductAuthRouteFailure> {
    RawCallbackValue::new(format!("{prefix}-{}-{}", Uuid::new_v4(), Uuid::new_v4()))
        .map_err(|_| ProductAuthRouteFailure::invalid_request())
}

fn parse_google_requested_scopes(
    raw_scopes: &[String],
) -> Result<Vec<ProviderScope>, ProductAuthRouteFailure> {
    if raw_scopes.is_empty() {
        return Err(ProductAuthRouteFailure::invalid_request());
    }
    raw_scopes
        .iter()
        .map(|scope| {
            if !is_allowed_google_scope(scope) {
                return Err(ProductAuthRouteFailure::invalid_request());
            }
            ProviderScope::new(scope.clone())
                .map_err(|_| ProductAuthRouteFailure::invalid_request())
        })
        .collect()
}

fn parse_google_callback_scopes(
    raw: Option<&str>,
) -> Result<Option<Vec<ProviderScope>>, ProductAuthRouteFailure> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.trim() != raw {
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    if raw.is_empty() {
        return Ok(Some(Vec::new()));
    }
    raw.split([' ', ','])
        .filter(|scope| !scope.is_empty())
        .map(|scope| {
            ProviderScope::new(scope.to_string())
                .map_err(|_| ProductAuthRouteFailure::malformed_callback())
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn is_allowed_google_scope(scope: &str) -> bool {
    matches!(
        scope,
        GOOGLE_CALENDAR_READONLY_SCOPE
            | GOOGLE_CALENDAR_EVENTS_SCOPE
            | GOOGLE_GMAIL_READONLY_SCOPE
            | GOOGLE_GMAIL_SEND_SCOPE
            | GOOGLE_GMAIL_MODIFY_SCOPE
    )
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
