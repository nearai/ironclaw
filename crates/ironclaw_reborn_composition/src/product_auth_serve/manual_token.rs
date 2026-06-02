//! Manual-token setup, submit, and secret-submit handlers.

use super::*;

pub(super) async fn manual_token_submit_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<ManualTokenSubmitRequest>,
) -> Result<Json<ManualTokenSubmitResponse>, ProductAuthRouteFailure> {
    let scope = scope_from_authenticated_caller_parts(
        &caller,
        &ScopeFields {
            session_id: request.session_id.clone(),
            thread_id: request.thread_id.clone(),
            invocation_id: None,
        },
    )?;
    let provider = AuthProviderId::new(request.provider)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let label = CredentialAccountLabel::new(request.account_label)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let token = request
        .token
        .into_validated()
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    // This legacy combined submit route creates a scoped credential and passes
    // the completed turn-gate continuation to product-auth dispatch. New callers
    // should prefer the setup + secret-submit split route, which binds
    // submissions to the invocation returned by setup.
    let continuation = AuthContinuationRef::TurnGateResume {
        turn_run_ref: TurnRunRef::new(request.run_id)
            .map_err(|_| ProductAuthRouteFailure::invalid_request())?,
        gate_ref: AuthGateRef::new(request.gate_ref)
            .map_err(|_| ProductAuthRouteFailure::invalid_request())?,
    };
    let expires_at = Utc::now() + ChronoDuration::seconds(PRODUCT_AUTH_FLOW_MAX_TTL_SECONDS);

    let challenge = run_with_backend_timeout(state.product_auth.request_manual_token_setup(
        RebornManualTokenSetupRequest::new(
            scope.clone(),
            provider,
            label,
            continuation,
            expires_at,
        ),
    ))
    .await?;
    let submitted = submit_manual_token_with_abandon(
        &state,
        &scope,
        challenge.interaction_id,
        token.into_secret(),
    )
    .await?;

    Ok(Json(ManualTokenSubmitResponse {
        credential_ref: submitted.account_id,
        status: submitted.status,
        continuation: submitted.continuation,
    }))
}

pub(super) async fn abandon_manual_token_after_submit_failure(
    state: &ProductAuthRouteState,
    scope: &AuthProductScope,
    interaction_id: AuthInteractionId,
    submit_error_code: AuthErrorCode,
) {
    match tokio::time::timeout(
        PRODUCT_AUTH_BACKEND_TIMEOUT,
        state
            .product_auth
            .abandon_manual_token(scope, interaction_id),
    )
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(cleanup_error)) => {
            tracing::warn!(
                error_code = ?submit_error_code,
                cleanup_error_code = ?cleanup_error.code,
                "manual-token submit failed and interaction cleanup failed — interaction may be orphaned until TTL"
            );
        }
        Err(_) => {
            tracing::warn!(
                error_code = ?submit_error_code,
                cleanup_error_code = ?AuthErrorCode::BackendUnavailable,
                "manual-token submit failed and interaction cleanup timed out — interaction may be orphaned until TTL"
            );
        }
    }
}

/// Submit a manual token and abandon the pending interaction on any submit
/// failure so failed setup attempts do not leave an active challenge behind.
pub(super) async fn submit_manual_token_with_abandon(
    state: &ProductAuthRouteState,
    scope: &AuthProductScope,
    interaction_id: AuthInteractionId,
    secret: SecretString,
) -> Result<RebornManualTokenSubmitResponse, ProductAuthRouteFailure> {
    match tokio::time::timeout(
        PRODUCT_AUTH_BACKEND_TIMEOUT,
        state
            .product_auth
            .submit_manual_token(RebornManualTokenSubmitRequest::new(
                scope.clone(),
                interaction_id,
                secret,
            )),
    )
    .await
    {
        Ok(Ok(submitted)) => Ok(submitted),
        Ok(Err(error)) => {
            let code = error.code;
            abandon_manual_token_after_submit_failure(state, scope, interaction_id, code).await;
            Err(ProductAuthRouteFailure::from(error))
        }
        Err(_) => {
            abandon_manual_token_after_submit_failure(
                state,
                scope,
                interaction_id,
                AuthErrorCode::BackendUnavailable,
            )
            .await;
            Err(ProductAuthRouteFailure::backend_timeout())
        }
    }
}

pub(super) async fn manual_token_setup_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<ManualTokenSetupRequest>,
) -> Result<Json<ManualTokenSetupResponse>, ProductAuthRouteFailure> {
    let scope = scope_from_authenticated_caller_parts(&caller, &request.scope)?;
    let invocation_id = scope.resource.invocation_id;
    let provider = AuthProviderId::new(request.provider)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let label = CredentialAccountLabel::new(request.account_label)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let continuation =
        manual_token_continuation(request.run_id.as_deref(), request.gate_ref.as_deref())?;
    let expires_at = Utc::now() + ChronoDuration::seconds(PRODUCT_AUTH_FLOW_MAX_TTL_SECONDS);

    let challenge = run_with_backend_timeout(state.product_auth.request_manual_token_setup(
        RebornManualTokenSetupRequest::new(scope, provider, label, continuation, expires_at),
    ))
    .await?;

    Ok(Json(ManualTokenSetupResponse {
        interaction_id: challenge.interaction_id,
        provider: challenge.provider,
        label: challenge.label,
        expires_at: challenge.expires_at,
        invocation_id,
    }))
}

pub(super) async fn manual_token_secret_submit_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<ManualTokenSecretSubmitRequest>,
) -> Result<Json<ManualTokenSubmitResponse>, ProductAuthRouteFailure> {
    // Secret-submit is the secure out-of-band entry point: the raw token is
    // read straight off the dedicated body, never echoed back, and never
    // surfaced in model transcript or tool arguments. Only the redacted
    // submit projection is returned.
    // invocation_id is required: it must be the id returned by setup so the
    // interaction service can match the pending scope.
    let scope =
        scope_from_authenticated_caller_parts_requiring_invocation(&caller, &request.scope)?;
    let interaction_id = parse_interaction_id(&request.interaction_id)?;
    // Validate the token before any async work. On validation failure, abandon
    // the interaction so it does not remain active until its TTL expires.
    let token = match request.token.into_validated() {
        Ok(t) => t,
        Err(_) => {
            abandon_manual_token_after_submit_failure(
                &state,
                &scope,
                interaction_id,
                AuthErrorCode::InvalidRequest,
            )
            .await;
            return Err(ProductAuthRouteFailure::invalid_request());
        }
    };

    let submitted =
        submit_manual_token_with_abandon(&state, &scope, interaction_id, token.into_secret())
            .await?;

    Ok(Json(ManualTokenSubmitResponse {
        credential_ref: submitted.account_id,
        status: submitted.status,
        continuation: submitted.continuation,
    }))
}

pub(super) fn manual_token_continuation(
    run_id: Option<&str>,
    gate_ref: Option<&str>,
) -> Result<AuthContinuationRef, ProductAuthRouteFailure> {
    match (run_id, gate_ref) {
        (Some(run_id), Some(gate_ref)) => Ok(AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string())
                .map_err(|_| ProductAuthRouteFailure::invalid_request())?,
            gate_ref: AuthGateRef::new(gate_ref.to_string())
                .map_err(|_| ProductAuthRouteFailure::invalid_request())?,
        }),
        (None, None) => Ok(AuthContinuationRef::SetupOnly),
        // run_id without gate_ref (or vice versa) is rejected so we never
        // resume the wrong gate or fabricate a partial turn-resume.
        _ => Err(ProductAuthRouteFailure::invalid_request()),
    }
}
