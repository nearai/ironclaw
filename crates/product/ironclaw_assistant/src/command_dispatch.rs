//! Workflow-owned command admission and execution ports.
//!
//! `commands` owns the source-agnostic command model. This module owns the
//! authority-bearing workflow context and the service boundary that decides
//! whether a command may execute from that context.

use async_trait::async_trait;
use ironclaw_product_contracts::command::ProductCommandContext;
use ironclaw_product_contracts::inbound::{ProductRejection, ProductRejectionKind};
use ironclaw_product_contracts::surface::ProductSurfaceError;
use serde::{Deserialize, Serialize};

use crate::commands::ProductCommand;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductCommandAdmission {
    Allowed,
    Rejected(ProductRejection),
}

#[async_trait]
pub trait ProductCommandAdmissionService: Send + Sync {
    async fn admit(
        &self,
        context: &ProductCommandContext,
        command: &ProductCommand,
    ) -> Result<ProductCommandAdmission, ProductSurfaceError>;
}

/// Fail-closed admission service used until a host composition supplies concrete
/// source/auth policy.
pub struct RejectingProductCommandAdmissionService;

#[async_trait]
impl ProductCommandAdmissionService for RejectingProductCommandAdmissionService {
    async fn admit(
        &self,
        _context: &ProductCommandContext,
        command: &ProductCommand,
    ) -> Result<ProductCommandAdmission, ProductSurfaceError> {
        Ok(ProductCommandAdmission::Rejected(
            ProductRejection::permanent(
                ProductRejectionKind::PolicyDenied,
                format!("command routing unavailable: {}", command.name()),
            ),
        ))
    }
}
