//! Credential account list, select, recovery, and refresh handlers.

use super::*;

pub(super) async fn accounts_list_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(package_id): Path<String>,
    Json(request): Json<AccountsListRequest>,
) -> Result<Json<CredentialAccountListPage>, ProductAuthRouteFailure> {
    // The requester extension is derived from the trusted `{package_id}` URL
    // path segment, never from the browser body (mirrors
    // `extension_oauth_start_handler`). An invalid id is rejected before any
    // backend call.
    let requester_extension = parse_extension_id(&package_id)?;
    // invocation_id is required so the list is scoped to the caller's current
    // interaction context; omitting it would silently yield an empty page.
    let scope =
        scope_from_authenticated_caller_parts_requiring_invocation(&caller, &request.scope)?;
    let provider = AuthProviderId::new(request.provider)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;

    let mut list_request =
        CredentialAccountListRequest::new(scope, provider).for_extension(requester_extension);
    if let Some(cursor) = request.cursor.as_deref() {
        list_request = list_request.with_cursor(parse_credential_account_id(cursor)?);
    }
    if let Some(limit) = request.limit {
        list_request = list_request.with_limit(limit);
    }
    list_request
        .validate()
        .map_err(ProductAuthRouteFailure::from)?;

    let page =
        run_with_backend_timeout(state.product_auth.list_credential_accounts(list_request)).await?;

    Ok(Json(page))
}

pub(super) async fn accounts_select_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(package_id): Path<String>,
    Json(request): Json<AccountsSelectRequest>,
) -> Result<Json<CredentialAccountProjection>, ProductAuthRouteFailure> {
    // Requester extension derived from the trusted `{package_id}` segment.
    let requester_extension = parse_extension_id(&package_id)?;
    // invocation_id required: links the selection to the active auth interaction
    // so the service can validate grant scope; omitting it would silently create
    // an orphaned scope unbound to any pending gate.
    let scope =
        scope_from_authenticated_caller_parts_requiring_invocation(&caller, &request.scope)?;
    let provider = AuthProviderId::new(request.provider)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let account_id = parse_credential_account_id(&request.account_id)?;

    let choice_request = CredentialAccountChoiceRequest::new(scope, provider, account_id)
        .for_extension(requester_extension);

    let projection =
        run_with_backend_timeout(state.product_auth.select_credential_account(choice_request))
            .await?;

    Ok(Json(projection))
}

pub(super) async fn accounts_recovery_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(package_id): Path<String>,
    Json(request): Json<AccountsRecoveryRequest>,
) -> Result<Json<CredentialRecoveryProjection>, ProductAuthRouteFailure> {
    // Requester extension derived from the trusted `{package_id}` segment.
    let requester_extension = parse_extension_id(&package_id)?;
    // invocation_id required: recovery projection is scoped to the active
    // interaction context; omitting it would scope to a fresh, unmatched invocation.
    let scope =
        scope_from_authenticated_caller_parts_requiring_invocation(&caller, &request.scope)?;
    let provider = AuthProviderId::new(request.provider)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;

    let recovery_request =
        CredentialRecoveryRequest::new(scope, provider).for_extension(requester_extension);

    let projection = run_with_backend_timeout(
        state
            .product_auth
            .project_credential_recovery(recovery_request),
    )
    .await?;

    Ok(Json(projection))
}

pub(super) async fn accounts_refresh_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(package_id): Path<String>,
    Json(request): Json<AccountsRefreshRequest>,
) -> Result<Json<CredentialRefreshReport>, ProductAuthRouteFailure> {
    // Requester extension derived from the trusted `{package_id}` segment.
    let requester_extension = parse_extension_id(&package_id)?;
    // invocation_id required: refresh is scoped to the active interaction.
    let scope =
        scope_from_authenticated_caller_parts_requiring_invocation(&caller, &request.scope)?;
    let provider = AuthProviderId::new(request.provider)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let account_id = parse_credential_account_id(&request.account_id)?;

    let refresh_request = CredentialRefreshRequest::new(scope, provider, account_id)
        .for_extension(requester_extension);

    let report = run_with_backend_timeout(
        state
            .product_auth
            .refresh_credential_account(refresh_request),
    )
    .await?;

    Ok(Json(report))
}
