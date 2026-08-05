//! The package-lifecycle product service port (PROPOSAL §6.1.3).
//!
//! [`crate::package_lifecycle`] owns the lifecycle *values*; this module owns
//! the service that answers in them. The split matters because the only
//! production implementation lives **outside** product, in
//! `ironclaw_extension_manager` (WS2.4) — which calls the lifecycle
//! *authority* in `ironclaw_extension_host`, the crate that may write lifecycle
//! state — while product and every transport call it through this port.
//!
//! Never here: any lifecycle authority, install policy, or service
//! implementation (including the unsupported-runtime fallback, which is
//! product's).

use async_trait::async_trait;
use ironclaw_host_api::ids::{AgentId, ExtensionId, ProjectId, TenantId, UserId};
use serde::Serialize;

use crate::command::ProductCommandContext;
use crate::package_lifecycle::{
    LifecyclePackageRef, LifecycleProductAction, LifecycleProductResponse,
};
use crate::surface::ProductSurfaceError;

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
    /// The default refuses with `Unavailable`/503: the bundle importer is a
    /// runtime capability a deployment either wired or did not, and a caller
    /// who uploaded a perfectly good zip did nothing wrong. 400 would blame
    /// them for it. ✎ **Flipped 2026-08-02 (CHECKLIST WS2, the "four WS2.1
    /// follow-ups" row) from `InvalidRequest`/400**, which the WS2.1 move had
    /// carried verbatim against its own prose;
    /// `bundle_import_defaults_to_service_unavailable` here and
    /// `webui_extension_import_reports_unavailable_when_no_service_is_wired`
    /// in `ironclaw_assistant`'s surface contract pin the code at both tiers so
    /// it cannot drift back silently.
    async fn import_extension_bundle(
        &self,
        _context: LifecycleProductContext,
        _bundle: Vec<u8>,
    ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
        Err(ProductSurfaceError::unavailable(false))
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
    /// ✎ **Key retyped `String` → `ExtensionId` 2026-08-02** (CHECKLIST WS2,
    /// the "four WS2.1 follow-ups" row): the doc always said "keyed by
    /// extension id" and every consumer looks the key up with an `ExtensionId`
    /// in hand, so the `String` only bought a place for a package id, a
    /// display name, or an installation id to be keyed in undetected.
    async fn installed_activation_errors(
        &self,
        _context: LifecycleProductContext,
    ) -> Result<std::collections::HashMap<ExtensionId, String>, ProductSurfaceError> {
        Ok(std::collections::HashMap::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_contracts::state::InstallationState;

    use crate::package_lifecycle::LifecyclePackageKind;
    use crate::surface::ProductSurfaceErrorCode;

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

    /// A service without bundle support must *refuse*, never return success —
    /// and it must refuse as an unwired capability (503), not as a malformed
    /// request (400): the bytes were never inspected, so nothing about the
    /// caller's request has been judged. The status code is asserted beside
    /// the error code because the WebUI maps the code straight onto the HTTP
    /// response, and a code/status pair that disagrees is how a 503 reaches a
    /// browser wearing a 400.
    #[tokio::test]
    async fn bundle_import_defaults_to_service_unavailable() {
        let error = MinimalLifecycleService
            .import_extension_bundle(surface_context(), vec![0x50, 0x4b])
            .await
            .expect_err("a service that does not implement bundle import must refuse");
        assert_eq!(error.code, ProductSurfaceErrorCode::Unavailable);
        assert_eq!(error.status_code, 503);
        assert!(
            !error.retryable,
            "an unwired capability does not become wired by retrying"
        );
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
