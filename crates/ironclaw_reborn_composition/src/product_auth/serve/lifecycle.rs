//! Credential lifecycle cleanup handler.

use super::*;

pub(super) async fn lifecycle_cleanup_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<LifecycleCleanupRequest>,
) -> Result<Json<SecretCleanupReport>, ProductAuthRouteFailure> {
    let scope = scope_from_authenticated_caller_parts(&caller, &request.scope)?;
    let extension_id = parse_extension_id(&request.extension_id)?;

    let cleanup_request = SecretCleanupRequest {
        scope,
        extension_id,
        action: request.action,
    };

    let report = run_with_backend_timeout(
        state
            .product_auth
            .cleanup_credentials_for_lifecycle(cleanup_request),
    )
    .await?;

    Ok(Json(report))
}
