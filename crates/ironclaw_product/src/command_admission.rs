//! Production admission policy for commands arriving from external channels.
//!
//! Commands execute with the paired user's authority, so the first shipping
//! policy is deliberately conservative: only direct conversations may invoke
//! them. Pairing and binding ownership are enforced by the ingress and
//! workflow layers on either side of this policy.

use async_trait::async_trait;
use ironclaw_host_api::ProductSurfaceError;

use crate::binding::route_kind_for_trigger;
use crate::command_dispatch::{
    ProductCommandAdmission, ProductCommandAdmissionService, ProductCommandContext,
};
use crate::commands::ProductCommand;
use crate::{ProductConversationRouteKind, ProductRejection, ProductRejectionKind};

/// Admit commands only from direct conversations.
pub struct DirectConversationCommandAdmission;

#[async_trait]
impl ProductCommandAdmissionService for DirectConversationCommandAdmission {
    async fn admit(
        &self,
        context: &ProductCommandContext,
        _command: &ProductCommand,
    ) -> Result<ProductCommandAdmission, ProductSurfaceError> {
        if route_kind_for_trigger(context.trigger) != ProductConversationRouteKind::Direct {
            return Ok(ProductCommandAdmission::Rejected(
                ProductRejection::permanent(
                    ProductRejectionKind::PolicyDenied,
                    "commands are limited to direct conversations",
                ),
            ));
        }
        Ok(ProductCommandAdmission::Allowed)
    }
}
