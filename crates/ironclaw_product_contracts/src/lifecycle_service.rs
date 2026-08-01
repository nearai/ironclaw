//! The package-lifecycle product service port (PROPOSAL §6.1.3).
//!
//! [`crate::package_lifecycle`] owns the lifecycle *values*; this module owns
//! the service that answers in them. The split matters because the only
//! production implementation lives **below** product, in
//! `ironclaw_extension_host` — it is the crate that may write lifecycle state —
//! while product and every transport call it through this port.
//!
//! Never here: any lifecycle authority, install policy, or service
//! implementation (including the unsupported-runtime fallback, which is
//! product's).

use async_trait::async_trait;
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};
use serde::Serialize;

use crate::command::ProductCommandContext;
use crate::package_lifecycle::{
    LifecyclePackageRef, LifecycleProductAction, LifecycleProductResponse,
};
use crate::surface::{ProductSurfaceError, ProductSurfaceErrorCode};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LifecycleProductSurfaceContext {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum LifecycleProductContext {
    Command(Box<ProductCommandContext>),
    Surface(LifecycleProductSurfaceContext),
}

#[async_trait]
pub trait LifecycleProductService: Send + Sync {
    async fn execute(
        &self,
        context: LifecycleProductContext,
        action: LifecycleProductAction,
    ) -> Result<LifecycleProductResponse, ProductSurfaceError>;

    async fn project_package(
        &self,
        context: LifecycleProductContext,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductSurfaceError>;

    /// Import a standalone extension from an uploaded bundle (zip bytes) — the
    /// WebUI "Install Tool" path. Only the local runtime service implements it.
    ///
    /// The default refuses with `InvalidRequest`/400. ✎ **Known mismatch,
    /// carried verbatim by the WS2.1 move and deliberately not changed in a
    /// move-shaped PR**: the wording that used to sit here said "unavailable",
    /// which is a different code (503) and a different meaning — 400 says the
    /// caller's request was malformed, and an unwired capability is not the
    /// caller's fault. Changing it changes an HTTP status on a live route, so
    /// it belongs in its own change with the WebUI path re-checked;
    /// `bundle_import_defaults_to_an_invalid_request_rather_than_silently_succeeding`
    /// pins today's behavior so the flip cannot happen silently. The flip —
    /// and this test's assertion with it — is owned by the CHECKLIST WS2 row
    /// "The four verbatim-relocated signatures WS2.1 deferred", which sizes it
    /// as the behavior change it is rather than as a signature correction.
    async fn import_extension_bundle(
        &self,
        _context: LifecycleProductContext,
        _bundle: Vec<u8>,
    ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
        Err(ProductSurfaceError::from_status(
            ProductSurfaceErrorCode::InvalidRequest,
            400,
            false,
        ))
    }

    /// Redacted activation error for each installed extension whose activation
    /// failed, keyed by extension id — sourced from the durable installation
    /// record's typed `last_error`. The extensions-list service threads this
    /// into `RebornExtensionInfo::activation_error` so a failed extension shows
    /// *why* it failed instead of collapsing to a bare `installed`/`failed`
    /// state with no reason.
    ///
    /// Default: none. A service that does not surface durable installation
    /// errors reports no reason and the wire's `activation_error` stays absent;
    /// the production extension-host service overrides this to read the
    /// installation records' `last_error`.
    ///
    /// ✎ **Known stringly-typed signature, carried verbatim by the WS2.1
    /// move.** The key should be `ExtensionId`, not `String` — the host
    /// implementation already holds a typed `ExtensionId` and stringifies only
    /// to satisfy this signature. Retyping it changes the contract for every
    /// implementor, which PLAN operating principle 2 keeps out of a move-shaped
    /// PR; it is owned by the CHECKLIST WS2 row "The four verbatim-relocated
    /// signatures WS2.1 deferred". It is independent of the
    /// `ProductSurfaceFailure` narrowing — `ExtensionId` is `host_api` vocabulary
    /// this crate may already name — so it need not wait on that slice.
    async fn installed_activation_errors(
        &self,
        _context: LifecycleProductContext,
    ) -> Result<std::collections::HashMap<String, String>, ProductSurfaceError> {
        Ok(std::collections::HashMap::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_contracts::state::InstallationState;

    use crate::package_lifecycle::LifecyclePackageKind;

    /// A service that implements only the two required methods, so the two
    /// defaults below are exercised as written rather than through an
    /// override. Fail-closed defaults are the reason they exist.
    struct MinimalLifecycleService;

    fn surface_context() -> LifecycleProductContext {
        LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
            tenant_id: TenantId::new("tenant-1").expect("valid tenant"),
            user_id: UserId::new("user-1").expect("valid user"),
            agent_id: None,
            project_id: None,
        })
    }

    fn package_ref() -> LifecyclePackageRef {
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack").expect("valid ref")
    }

    #[async_trait]
    impl LifecycleProductService for MinimalLifecycleService {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            _action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            Ok(LifecycleProductResponse::projection(
                Some(package_ref()),
                InstallationState::Active,
                Vec::new(),
            ))
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            Ok(LifecycleProductResponse::projection(
                Some(package_ref),
                InstallationState::Active,
                Vec::new(),
            ))
        }
    }

    #[tokio::test]
    async fn the_two_required_methods_have_no_default_and_answer_in_lifecycle_values() {
        let service: &dyn LifecycleProductService = &MinimalLifecycleService;
        let executed = service
            .execute(
                surface_context(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref(),
                },
            )
            .await
            .expect("execute is required, not defaulted");
        assert_eq!(executed.phase, InstallationState::Active);

        let projected = service
            .project_package(surface_context(), package_ref())
            .await
            .expect("project_package is required, not defaulted");
        assert_eq!(projected.package_ref, Some(package_ref()));
    }

    /// Pins today's behavior, not the desired one — see the mismatch note on
    /// `import_extension_bundle`. The point that matters either way: a service
    /// without bundle support must *refuse*, never return success.
    #[tokio::test]
    async fn bundle_import_defaults_to_an_invalid_request_rather_than_silently_succeeding() {
        let error = MinimalLifecycleService
            .import_extension_bundle(surface_context(), vec![0x50, 0x4b])
            .await
            .expect_err("a service that does not implement bundle import must refuse");
        assert_eq!(error.code, ProductSurfaceErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn activation_errors_default_to_none_so_the_wire_field_stays_absent() {
        let errors = MinimalLifecycleService
            .installed_activation_errors(surface_context())
            .await
            .expect("default reports no durable errors");
        assert!(errors.is_empty());
    }
}
