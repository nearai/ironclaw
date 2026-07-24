//! Product-facing lifecycle contract for Reborn package UX.
//!
//! This module deliberately models package/install lifecycle separately from
//! auth, approval, pairing, and policy gates. Those remain owned by their
//! dedicated services; lifecycle projections may only carry redacted refs to
//! the owning interaction.

use async_trait::async_trait;
use ironclaw_host_api::{
    AgentId, InstallationState, ProductSurfaceError, ProductSurfaceErrorCode, ProjectId, TenantId,
    UserId,
};
use serde::Serialize;

use crate::ProductCommandContext;

pub use ironclaw_host_api::{
    ChannelConnectionRequirement, LifecycleBlockerRef, LifecycleChannelDirections,
    LifecycleCommandKind, LifecycleExtensionCredentialRequirement,
    LifecycleExtensionCredentialSetup, LifecycleExtensionOnboarding, LifecycleExtensionRuntimeKind,
    LifecycleExtensionSource, LifecycleExtensionSummary, LifecycleInstallScope,
    LifecycleInstalledExtensionSummary, LifecyclePackageId, LifecyclePackageKind,
    LifecyclePackageRef, LifecycleProductAction, LifecycleProductPayload, LifecycleProductResponse,
    LifecycleReadinessBlocker, LifecycleSearchExtensionSummary, LifecycleSkillSource,
    LifecycleSkillSummary,
};

const LIFECYCLE_REF_MAX_BYTES: usize = 512;

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
    /// WebUI "Install Tool" path. Default is unavailable; only the local runtime
    /// service implements it.
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
    async fn installed_activation_errors(
        &self,
        _context: LifecycleProductContext,
    ) -> Result<std::collections::HashMap<String, String>, ProductSurfaceError> {
        Ok(std::collections::HashMap::new())
    }
}

#[derive(Debug, Clone)]
pub struct UnsupportedLifecycleProductService {
    runtime_ref: String,
}

impl UnsupportedLifecycleProductService {
    pub fn new(runtime_ref: impl Into<String>) -> Result<Self, ProductSurfaceError> {
        Ok(Self {
            runtime_ref: validate_lifecycle_string(
                runtime_ref.into(),
                "unsupported lifecycle runtime ref",
                LIFECYCLE_REF_MAX_BYTES,
            )?,
        })
    }

    pub fn new_static(runtime_ref: &'static str) -> Self {
        debug_assert!(
            validate_lifecycle_string(
                runtime_ref.to_string(),
                "unsupported lifecycle runtime ref",
                LIFECYCLE_REF_MAX_BYTES,
            )
            .is_ok()
        );
        Self {
            runtime_ref: runtime_ref.to_string(),
        }
    }

    fn unsupported_projection(
        &self,
        package_ref: Option<LifecyclePackageRef>,
    ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
        Ok(LifecycleProductResponse::projection(
            package_ref,
            InstallationState::Unsupported,
            vec![
                LifecycleReadinessBlocker::runtime(Some(self.runtime_ref.clone()))
                    .map_err(ProductSurfaceError::internal_from)?,
            ],
        ))
    }
}

#[async_trait]
impl LifecycleProductService for UnsupportedLifecycleProductService {
    async fn execute(
        &self,
        _context: LifecycleProductContext,
        action: LifecycleProductAction,
    ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
        self.unsupported_projection(action.package_ref().cloned())
    }

    async fn project_package(
        &self,
        _context: LifecycleProductContext,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
        self.unsupported_projection(Some(package_ref))
    }
}

/// Validates a lifecycle string: non-empty, within byte limit, with optional
/// control-character filtering.
fn validate_lifecycle_string(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, ProductSurfaceError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(lifecycle_invalid_request(label));
    }
    if value.len() > max_bytes {
        return Err(lifecycle_invalid_request(label));
    }
    if trimmed.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(lifecycle_invalid_request(label));
    }
    Ok(trimmed.to_string())
}

fn lifecycle_invalid_request(label: &'static str) -> ProductSurfaceError {
    tracing::debug!(field = label, "invalid lifecycle value");
    ProductSurfaceError::from_status(ProductSurfaceErrorCode::InvalidRequest, 400, false)
}
