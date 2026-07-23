//! OAuth start and callback handlers.
//!
//! One start path and one callback path serve every vendor: the `{provider}`
//! path parameter (and the start request's `provider` field) carry a vendor
//! id resolved to recipe DATA through the auth engine — there is no
//! per-vendor handler, descriptor, or branch. Post-exchange identity binding
//! is a per-vendor hook registered as data on the route state.

use super::*;

pub(super) async fn oauth_start_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<ProductSurfaceCaller>,
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
                pkce_verifier: pkce_verifier.clone(),
                update_binding: None,
                continuation: AuthContinuationRef::SetupOnly,
                expires_at: request.expires_at,
            }),
    )
    .await?;
    // Same-process fast path only; the durable per-flow copy written by
    // `start_setup_oauth_flow` is the source of truth across restarts.
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
    Extension(caller): Extension<ProductSurfaceCaller>,
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

/// Authenticated, caller-scoped retry of an OAuth flow's unacknowledged
/// internal continuation.
///
/// This command never repeats provider exchange. It exists for the browser's
/// origin-independent completion watcher: a transient runtime-discovery
/// failure can leave OAuth durably completed while readiness reconciliation is
/// still pending. Re-driving the exact durable continuation converges that
/// state without a public Activate action or another OAuth round-trip.
pub(super) async fn oauth_flow_reconcile_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<ProductSurfaceCaller>,
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
    let status =
        run_with_backend_timeout(state.product_auth.reconcile_oauth_flow(&scope, flow_id)).await?;
    Ok(Json(OAuthFlowStatusResponse { status }))
}

/// Recipe-driven extension OAuth start: the browser selects one opaque
/// manifest requirement key, then the server resolves that installed
/// extension's vendor, label, and scopes before the engine constructs the
/// authorization URL. The global recipe catalog remains only the provider
/// protocol/config ceiling; it is not extension authority.
pub(super) async fn extension_oauth_start_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Path(package_id): Path<String>,
    Json(request): Json<ExtensionOAuthStartRequest>,
) -> Result<Json<ProductOAuthStartResponse>, ProductAuthRouteFailure> {
    let requester_extension =
        ExtensionId::new(package_id).map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    // Fail closed before any flow work: an extension absent from the
    // caller's installed inventory must not mint an OAuth flow.
    state
        .require_installed_extension(&caller, &requester_extension)
        .await?;
    let now = Utc::now();
    if request.expires_at <= now
        || request.expires_at > now + ChronoDuration::seconds(PRODUCT_AUTH_FLOW_MAX_TTL_SECONDS)
    {
        return Err(ProductAuthRouteFailure::invalid_request());
    }

    let requirement = state
        .resolve_extension_oauth_requirement(
            &caller,
            &requester_extension,
            request.requirement.as_str(),
        )
        .await?;
    let engine = state.auth_engine()?;
    let provider = AuthProviderId::new(requirement.provider)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let account_label = CredentialAccountLabel::new(requirement.account_label)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let requested_scopes = requirement
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
    let flow_id = AuthFlowId::new();
    let update_binding = scoped_update_binding_for_requester(
        &state,
        scope.clone(),
        provider.clone(),
        Some(&requester_extension),
    )
    .await?;
    let prepared = run_with_backend_timeout(engine.prepare_oauth_flow(
        ironclaw_auth::PrepareOAuthFlowRequest {
            vendor: provider.as_str().to_string(),
            scope: scope.clone(),
            flow_id,
            account_label,
            requested_scopes,
        },
    ))
    .await?;

    let flow = run_with_backend_timeout(
        state
            .product_auth
            .start_setup_oauth_flow(RebornOAuthStartFlowRequest {
                flow_id: Some(flow_id),
                scope: scope.clone(),
                provider: provider.clone(),
                authorization_url: prepared.authorization_url.clone(),
                opaque_state_hash: prepared.opaque_state_hash.clone(),
                pkce_verifier_hash: prepared.pkce_verifier_hash.clone(),
                pkce_verifier: prepared.pkce_verifier.clone(),
                update_binding,
                continuation: AuthContinuationRef::LifecycleActivation {
                    package_ref: ironclaw_auth::LifecyclePackageRef::new(
                        requester_extension.as_str(),
                    )
                    .map_err(|_| ProductAuthRouteFailure::invalid_request())?,
                },
                expires_at: request.expires_at,
            }),
    )
    .await?;
    // Same-process fast path only; the durable per-flow copy written by
    // `start_setup_oauth_flow` is the source of truth across restarts.
    state.store_pkce_verifier(flow.id, prepared.pkce_verifier.clone(), flow.expires_at)?;

    let response = ProductOAuthStartResponse {
        flow_id: flow.id,
        status: flow.status,
        provider,
        authorization_url: prepared.authorization_url,
        expires_at: flow.expires_at,
        continuation: flow.continuation,
        callback_scope: scope_hint(&scope),
    };
    // Close the start-vs-removal race: a concurrent uninstall cancels the
    // pending flows it can see, so a flow minted after that sweep would
    // survive it. Re-check the inventory and abort the just-started flow
    // (cancel + drop its process-local PKCE verifier) when the extension
    // is gone, so a late callback cannot complete it.
    if let Err(error) = state
        .require_installed_extension(&caller, &requester_extension)
        .await
    {
        run_with_backend_timeout(
            state
                .product_auth
                .flow_manager()
                .cancel_flow(&scope, response.flow_id),
        )
        .await?;
        state
            .forget_pkce_verifier_everywhere(&scope, response.flow_id)
            .await;
        return Err(error);
    }
    Ok(Json(response))
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
    let scope = scope_from_callback_query(&state, &query)?;
    let state_hash = opaque_state_hash(state_value.as_str())?;

    let flow_provider = if is_authorized_callback_candidate(&query) {
        Some(
            run_with_backend_timeout(state.product_auth.ensure_oauth_callback_flow_known(
                &scope,
                flow_id,
                &state_hash,
            ))
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
            scope: scope.clone(),
            flow_id,
            opaque_state_hash: state_hash,
            outcome,
        },
    ))
    .await
    {
        Ok(response) => {
            state.forget_pkce_verifier_everywhere(&scope, flow_id).await;
            response
        }
        Err(error) => {
            if should_forget_pkce_verifier(error.body.code) {
                state.forget_pkce_verifier_everywhere(&scope, flow_id).await;
            }
            return Err(error);
        }
    };

    Ok(oauth_callback_response(&headers, response))
}

/// One OAuth callback handler for every vendor at the static
/// `/api/reborn/product-auth/oauth/{provider}/callback` path (AUTH-13).
///
/// Safety-preserving invariants (identical for every vendor): the raw `state`
/// is hashed once and claimed through `AuthFlowManager` (CSRF/state-hash +
/// single-use/replay), the PKCE verifier is resolved from the process-local
/// cache then the durable gate store, vendor tokens are exchanged only after
/// the flow is claimed, the callback tenant must match the route tenant
/// before any exchange, and the flow's vendor must match the `{provider}`
/// path (cross-vendor callbacks rejected).
pub(super) async fn vendor_oauth_callback_handler(
    State(state): State<ProductAuthRouteState>,
    Path(provider): Path<String>,
    RawQuery(raw_query): RawQuery,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ProductAuthRouteFailure> {
    // Browser popups (Accept: text/html) must never see a bare JSON route
    // failure: render the failure page instead, which emits the cross-window
    // "failed" completion signal (with the flow id once the state decoded) and
    // closes the popup so the parent surface can show a retryable error.
    let mut known_flow_id = None;
    match vendor_oauth_callback_attempt(
        state,
        provider,
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

async fn vendor_oauth_callback_attempt(
    state: ProductAuthRouteState,
    provider: String,
    raw_query: Option<String>,
    uri: Uri,
    headers: &HeaderMap,
    known_flow_id: &mut Option<AuthFlowId>,
) -> Result<Response, ProductAuthRouteFailure> {
    validate_callback_raw_query(raw_query.as_deref())?;
    // `{provider}` is data: it must resolve to an active vendor recipe before
    // anything else happens.
    let provider =
        AuthProviderId::new(provider).map_err(|_| ProductAuthRouteFailure::malformed_callback())?;
    let engine = state.auth_engine()?;
    if engine
        .recipes()
        .recipe_for_vendor(provider.as_str())
        .is_none()
    {
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    let query = axum::extract::Query::<VendorOAuthCallbackQuery>::try_from_uri(&uri)
        .map_err(|_| ProductAuthRouteFailure::malformed_callback())?
        .0;
    validate_vendor_callback_query_fields(&query)?;
    let state_value = query
        .state
        .as_ref()
        .ok_or_else(ProductAuthRouteFailure::malformed_callback)?;
    let state_hash = opaque_state_hash(state_value.as_str())?;
    let callback_state =
        OAuthCallbackState::decode(OAuthCallbackStateKind::RECIPE, state_value.as_str())
            .map_err(ProductAuthRouteFailure::from)?;
    let flow_id = callback_state.flow_id();
    *known_flow_id = Some(flow_id);
    let callback_scope = callback_state.scope();
    // Reject a callback state minted for another tenant before any vendor
    // exchange.
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
        state
            .forget_pkce_verifier_everywhere(callback_scope, flow_id)
            .await;
        return oauth_callback_route_result_response(headers, response);
    }

    let flow_provider = match run_with_backend_timeout(
        state
            .product_auth
            .ensure_oauth_callback_flow_known(callback_scope, flow_id, &state_hash),
    )
    .await
    {
        Ok(flow_provider) => flow_provider,
        Err(error) => {
            state.remove_pkce_verifier(flow_id);
            return Err(error);
        }
    };
    // Cross-vendor rejection: a state minted for one vendor's flow cannot
    // complete through another vendor's callback path.
    if flow_provider != provider {
        state.remove_pkce_verifier(flow_id);
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
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
    // Vendor-echoed granted scopes: when the redirect carries a scope list it
    // must include everything requested (else the user narrowed the consent —
    // denied); when absent the requested scopes ride to the exchange, where
    // the token-response scope extraction applies the recipe's rule.
    let callback_scopes =
        match resolve_callback_scopes(callback_state.requested_scopes(), query.scopes.as_deref()) {
            Ok(CallbackScopeOutcome::Scopes(scopes)) => scopes,
            Ok(CallbackScopeOutcome::ProviderDenied) => {
                state
                    .forget_pkce_verifier_everywhere(callback_scope, flow_id)
                    .await;
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
                provider: provider.clone(),
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
    // Post-exchange identity binding is a vendor-blind hook registered as
    // data; it receives the callback's vendor id and resolves the rest.
    let identity_check = state.provider_identity_hook(provider.as_str(), callback_scope);
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
            state
                .forget_pkce_verifier_everywhere(callback_scope, flow_id)
                .await;
            response
        }
        Err(error) => {
            if should_forget_pkce_verifier(error.body.code) {
                state
                    .forget_pkce_verifier_everywhere(callback_scope, flow_id)
                    .await;
            }
            return Err(error);
        }
    };

    Ok(oauth_callback_response(headers, response))
}

enum CallbackScopeOutcome {
    Scopes(Vec<ProviderScope>),
    ProviderDenied,
}

/// Generic scope-echo rule: a vendor that echoes granted scopes on the
/// redirect must have granted everything requested; a vendor that echoes
/// nothing leaves scope resolution to the token response (recipe-driven).
fn resolve_callback_scopes(
    requested_scopes: &[ProviderScope],
    query_scopes: Option<&str>,
) -> Result<CallbackScopeOutcome, ProductAuthRouteFailure> {
    let Some(raw) = query_scopes else {
        return Ok(CallbackScopeOutcome::Scopes(requested_scopes.to_vec()));
    };
    if raw.trim() != raw {
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    if raw.is_empty() {
        return Ok(CallbackScopeOutcome::ProviderDenied);
    }
    let echoed = raw
        .split([' ', ','])
        .filter(|scope| !scope.is_empty())
        .map(|scope| {
            ProviderScope::new(scope.to_string())
                .map_err(|_| ProductAuthRouteFailure::malformed_callback())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if requested_scopes
        .iter()
        .all(|requested| echoed.iter().any(|scope| scope == requested))
    {
        Ok(CallbackScopeOutcome::Scopes(requested_scopes.to_vec()))
    } else {
        Ok(CallbackScopeOutcome::ProviderDenied)
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
            "This account is already connected to another Reborn user. Disconnect it from that other account, then try again."
        }
        AuthErrorCode::LifecycleActivationFailed => {
            "Authorization completed, but the extension could not finish setup. Return to IronClaw to review the extension's setup requirements."
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
        None => return Err(ProductAuthRouteFailure::malformed_callback()),
    };
    if flow_provider.is_some_and(|known_provider| known_provider != &provider) {
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    let account_label = match query.account_label.as_deref() {
        Some(account_label) => CredentialAccountLabel::new(account_label.to_string())
            .map_err(|_| ProductAuthRouteFailure::malformed_callback())?,
        None => return Err(ProductAuthRouteFailure::malformed_callback()),
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
    let scopes = parse_provider_scopes(query.scopes.as_deref())?;
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

fn validate_vendor_callback_query_fields(
    query: &VendorOAuthCallbackQuery,
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
    )
}
