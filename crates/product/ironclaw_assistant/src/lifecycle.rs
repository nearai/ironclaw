//! Product-facing lifecycle contract for Reborn package UX.
//!
//! This module deliberately models package/install lifecycle separately from
//! auth, approval, pairing, and policy gates. Those remain owned by their
//! dedicated services; lifecycle projections may only carry redacted refs to
//! the owning interaction.

use async_trait::async_trait;
use ironclaw_extension_contracts::state::InstallationState;
use ironclaw_product_contracts::lifecycle_service::{
    LifecycleProductContext, LifecycleProductService,
};
use ironclaw_product_contracts::surface::{ProductSurfaceError, ProductSurfaceErrorCode};

pub use ironclaw_extension_contracts::lifecycle_id::{LifecycleBlockerRef, LifecyclePackageId};
pub use ironclaw_product_contracts::package_lifecycle::{
    ChannelConnectionRequirement, LifecycleChannelDirections, LifecycleCommandKind,
    LifecycleExtensionCredentialRequirement, LifecycleExtensionCredentialSetup,
    LifecycleExtensionOnboarding, LifecycleExtensionRuntimeKind, LifecycleExtensionSource,
    LifecycleExtensionSummary, LifecycleInstallScope, LifecycleInstalledExtensionSummary,
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductAction, LifecycleProductPayload,
    LifecycleProductResponse, LifecycleReadinessBlocker, LifecycleSearchExtensionSummary,
    LifecycleSkillSource, LifecycleSkillSummary, project_public_lifecycle_states,
    public_lifecycle_response_json,
};

const LIFECYCLE_REF_MAX_BYTES: usize = 512;

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
