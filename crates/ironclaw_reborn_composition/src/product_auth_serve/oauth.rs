//! OAuth start and callback handlers.

use super::*;
use crate::oauth_dcr::DcrOAuthCallbackState;

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
        requested_scopes.clone(),
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

pub(super) async fn google_oauth_callback_handler(
    State(state): State<ProductAuthRouteState>,
    RawQuery(raw_query): RawQuery,
    uri: Uri,
    headers: HeaderMap,
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
        return response.map(|response| oauth_callback_response(&headers, response));
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
    let requested_scopes = callback_state.requested_scopes();
    let callback_scopes = match parse_google_callback_scopes(query.scopes.as_deref()) {
        Ok(Some(callback_scopes)) => {
            if let Err(error) = validate_google_callback_includes_requested_scopes(
                &callback_scopes,
                requested_scopes,
            ) {
                state.remove_pkce_verifier(flow_id);
                return Err(error);
            }
            requested_scopes.to_vec()
        }
        Ok(None) => requested_scopes.to_vec(),
        Err(error) => {
            state.remove_pkce_verifier(flow_id);
            return Err(ProductAuthRouteFailure::from(error));
        }
    };
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

    Ok(oauth_callback_response(&headers, response))
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
        return Err(ProductAuthRouteFailure::malformed_callback());
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

fn oauth_callback_completion_html(response: &RebornOAuthCallbackResponse) -> Response {
    const CHANNEL: &str = "ironclaw-product-auth";
    const STORAGE_KEY: &str = "ironclaw:product-auth:oauth-complete";
    const MESSAGE_TYPE: &str = "ironclaw:product-auth:oauth-complete";

    let payload = json!({
        "type": MESSAGE_TYPE,
        "flowId": response.flow_id,
        "status": response.status,
        "continuation": response.continuation,
    })
    .to_string();
    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Authorization complete</title>
</head>
<body>
  <p>Authorization complete. You can close this window.</p>
  <script>
    (() => {{
      const payload = {payload};
      try {{
        new BroadcastChannel("{CHANNEL}").postMessage(payload);
      }} catch (_err) {{}}
      try {{
        localStorage.setItem(
          "{STORAGE_KEY}",
          JSON.stringify({{ ...payload, completedAt: Date.now() }})
        );
      }} catch (_err) {{}}
      window.close();
    }})();
  </script>
</body>
</html>"#
    );
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
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
    use crate::oauth_gate::{GoogleOAuthGateProvider, GoogleOAuthGateProviderRegistry};
    use async_trait::async_trait;
    use ironclaw_auth::{GOOGLE_CALENDAR_READONLY_SCOPE, InMemoryAuthProductServices};
    use ironclaw_host_api::{RuntimeCredentialAccountProviderId, RuntimeCredentialAuthRequirement};
    use ironclaw_secrets::{InMemorySecretStore, SecretStore};
    use ironclaw_turns::{TurnRunId, TurnScope};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn google_oauth_callback_uses_gate_pkce_store_when_route_cache_misses() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let secret_store = Arc::new(InMemorySecretStore::new());
        let secret_store_for_provider: Arc<dyn SecretStore> = secret_store.clone();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let google_gate = Arc::new(GoogleOAuthGateProvider::new(
            OAuthClientConfig::new(
                "google-client.apps.googleusercontent.com",
                "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
                None,
            )
            .expect("google oauth client"),
            secret_store_for_provider,
        ));
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), dispatcher.clone())
                .with_flow_record_source(shared)
                .with_oauth_gate_registry(Arc::new(GoogleOAuthGateProviderRegistry::new(vec![
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
}
