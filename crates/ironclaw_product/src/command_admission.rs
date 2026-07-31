//! Production admission policy for commands arriving from external channels.
//!
//! Commands execute with the paired user's authority, so the first shipping
//! policy is deliberately conservative: only direct conversations may invoke
//! them. Pairing and binding ownership are enforced by the ingress and
//! workflow layers on either side of this policy. Admin-audience commands
//! additionally require the bound actor to clear the admin-users role
//! boundary — see [`CommandActorRoleResolver`].

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::product_surface::ProductSurfaceError;

use crate::binding::route_kind_for_trigger;
use crate::command_dispatch::{
    ProductCommandAdmission, ProductCommandAdmissionService, ProductCommandContext,
};
use crate::commands::{
    CommandAudience, ProductCommand, UnknownProductCommandName, declared_command_help_text,
    required_audience, validate_declared_product_command,
};
use crate::reborn_services::AdminUserRole;
use crate::{ProductConversationRouteKind, ProductRejection, ProductRejectionKind};

/// Resolves the admin-boundary role of the ACTIVE bound user behind an
/// inbound channel actor. `Ok(None)` means unbound actor, missing record, or
/// suspended account — all treated as not-admin (fail closed). `Err` means
/// transient resolution failure; the command fails retryable rather than
/// silently degrading to member or admin treatment.
#[async_trait]
pub trait CommandActorRoleResolver: Send + Sync {
    async fn actor_role(
        &self,
        context: &ProductCommandContext,
    ) -> Result<Option<AdminUserRole>, ProductSurfaceError>;
}

/// Admit only manifest-enabled commands from direct conversations, then gate
/// admin-audience commands on the actor's admin-users role.
pub struct DirectConversationCommandAdmission {
    allowed_commands: BTreeSet<String>,
    roles: Arc<dyn CommandActorRoleResolver>,
}

impl DirectConversationCommandAdmission {
    pub fn new<I, S>(
        commands: I,
        roles: Arc<dyn CommandActorRoleResolver>,
    ) -> Result<Self, UnknownProductCommandName>
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
        Ok(Self {
            allowed_commands,
            roles,
        })
    }
}

#[async_trait]
impl ProductCommandAdmissionService for DirectConversationCommandAdmission {
    async fn admit(
        &self,
        context: &ProductCommandContext,
        command: &ProductCommand,
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
                    // Internal-only `ProductRejection.reason`: the observer never
                    // echoes it (it renders its own role-filtered, prefix-aware
                    // `command_help_text` instead), so this stays the bare
                    // `/{name}` form rather than threading the channel's display
                    // prefix through admission.
                    declared_command_help_text(&self.allowed_commands),
                ),
            ));
        }
        if required_audience(command) == CommandAudience::Admin {
            let role = self.roles.actor_role(context).await?;
            if !role.is_some_and(AdminUserRole::is_admin) {
                return Ok(ProductCommandAdmission::Rejected(
                    ProductRejection::permanent(
                        ProductRejectionKind::AccessDenied,
                        "admin-audience command from a non-admin actor",
                    ),
                ));
            }
        }
        Ok(ProductCommandAdmission::Allowed)
    }
}
