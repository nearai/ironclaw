//! Credential lifecycle cleanup handler.
//!
//! Extension-keyed cleanup only. The request is built WITHOUT a provider
//! selector, so this route revokes/deactivates the extension's OWN accounts,
//! strips its grants, and (on uninstall) cancels the extension's own
//! `LifecycleActivation` flows via the package selector — but it never
//! selects the caller's reusable per-provider personal credentials and never
//! cancels other flows on the provider. Full uninstall cleanup (exclusive
//! personal-credential revocation) lives in
//! `extension_host/extension_lifecycle.rs::revoke_exclusive_credentials`.
//! Canceled turn-gate denials and setup-PKCE-verifier deletion are handled
//! server-side inside `cleanup_credentials_for_lifecycle` before the report
//! is serialized; the `#[serde(skip)]` handoff fields never cross this
//! boundary.

use super::*;

pub(super) async fn lifecycle_cleanup_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Json(request): Json<LifecycleCleanupRequest>,
) -> Result<Json<SecretCleanupReport>, ProductAuthRouteFailure> {
    let scope = scope_from_authenticated_caller_parts(&caller, &request.scope)?;
    let extension_id = parse_extension_id(&request.extension_id)?;
    let lifecycle_package = ironclaw_auth::LifecyclePackageRef::new(extension_id.as_str())
        .map_err(|error| {
            tracing::debug!(%error, "lifecycle package ref rejected extension id");
            ProductAuthRouteFailure::invalid_request()
        })?;

    let cleanup_request = SecretCleanupRequest {
        scope,
        extension_id,
        provider: None,
        lifecycle_package: Some(lifecycle_package),
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
