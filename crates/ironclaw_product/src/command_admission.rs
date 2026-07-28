//! Production admission policy for commands arriving from external channels.
//!
//! Commands execute with the paired user's authority, so the first shipping
//! policy is deliberately conservative: only direct conversations may invoke
//! them. Pairing and binding ownership are enforced by the ingress and
//! workflow layers on either side of this policy.

use async_trait::async_trait;
use ironclaw_host_api::ProductSurfaceError;
use std::collections::BTreeSet;

use crate::binding::route_kind_for_trigger;
use crate::command_dispatch::{
    ProductCommandAdmission, ProductCommandAdmissionService, ProductCommandContext,
};
use crate::commands::{
    ProductCommand, UnknownProductCommandName, declared_command_help_text,
    validate_declared_product_command,
};
use crate::{ProductConversationRouteKind, ProductRejection, ProductRejectionKind};

/// Admit only manifest-enabled commands from direct conversations.
pub struct DirectConversationCommandAdmission {
    allowed_commands: BTreeSet<String>,
}

impl DirectConversationCommandAdmission {
    pub fn new<I, S>(commands: I) -> Result<Self, UnknownProductCommandName>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut allowed_commands = BTreeSet::new();
        for command in commands {
            let command = command.as_ref();
            validate_declared_product_command(command)?;
            allowed_commands.insert(command.to_string());
        }
        Ok(Self { allowed_commands })
    }
}

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
        if !self.allowed_commands.contains(&context.requested_command) {
            return Ok(ProductCommandAdmission::Rejected(
                ProductRejection::permanent(
                    ProductRejectionKind::InvalidRequest,
                    declared_command_help_text(&self.allowed_commands),
                ),
            ));
        }
        Ok(ProductCommandAdmission::Allowed)
    }
}
