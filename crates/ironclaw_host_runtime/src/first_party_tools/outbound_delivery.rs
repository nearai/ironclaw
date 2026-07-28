use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    CapabilityId, DispatchInputIssue, DispatchInputIssueCode, EffectKind, HostApiError,
    PermissionMode, ResourceUsage, RuntimeDispatchErrorKind,
};
use ironclaw_outbound::{
    OutboundDeliveryTargetId, RouteCurrentRunFinalReply, RouteCurrentRunFinalReplyError,
    RouteCurrentRunFinalReplyRequest,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

use super::{first_party_capability_manifest, resource_profile};

pub const OUTBOUND_DELIVERY_TARGET_ROUTE_CURRENT_CAPABILITY_ID: &str =
    "builtin.outbound_delivery_target_route_current";

const DESCRIPTION: &str = "Route only the final assistant answer for the current run to one opaque target id returned by builtin__outbound_delivery_targets_list. Use this when the user asks for this answer to appear in a particular channel or in the web app. This delivers IronClaw's answer to the same user; it must not be replaced by an integration send-message tool, which sends a message as the user to another person. For scheduled results, pass delivery_target_id to builtin__trigger_create instead.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteCurrentInput {
    target_id: OutboundDeliveryTargetId,
}

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        OUTBOUND_DELIVERY_TARGET_ROUTE_CURRENT_CAPABILITY_ID,
        DESCRIPTION,
        vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
        PermissionMode::Allow,
        resource_profile(),
    )
}

pub(super) fn insert_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    router: Arc<dyn RouteCurrentRunFinalReply>,
) -> Result<(), HostApiError> {
    registry.insert_handler(
        CapabilityId::new(OUTBOUND_DELIVERY_TARGET_ROUTE_CURRENT_CAPABILITY_ID)?,
        Arc::new(RouteCurrentHandler { router }),
    );
    Ok(())
}

struct RouteCurrentHandler {
    router: Arc<dyn RouteCurrentRunFinalReply>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for RouteCurrentHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let input: RouteCurrentInput =
            serde_json::from_value(request.input).map_err(|_| invalid_target_input())?;
        let run_id = request.run_id.ok_or_else(|| {
            FirstPartyCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::OperationFailed,
                "current-run delivery routing requires an active run",
            )
        })?;
        let actor = request.authenticated_actor_user_id.ok_or_else(|| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::PolicyDenied)
        })?;
        self.router
            .route_current_run_final_reply(RouteCurrentRunFinalReplyRequest {
                run_id,
                scope: request.scope,
                authenticated_actor_user_id: actor,
                target_id: input.target_id,
            })
            .await
            .map_err(map_route_error)?;
        Ok(FirstPartyCapabilityResult::new(
            json!({"routed": true}),
            ResourceUsage::default(),
        ))
    }
}

fn map_route_error(error: RouteCurrentRunFinalReplyError) -> FirstPartyCapabilityError {
    match error {
        RouteCurrentRunFinalReplyError::TargetUnavailable => invalid_target_input(),
        RouteCurrentRunFinalReplyError::InvalidRequest => {
            FirstPartyCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::OperationFailed,
                "the current run cannot accept an outbound delivery target",
            )
        }
        RouteCurrentRunFinalReplyError::AccessDenied => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::PolicyDenied)
        }
        RouteCurrentRunFinalReplyError::Unavailable | RouteCurrentRunFinalReplyError::Internal => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
        }
    }
}

fn invalid_target_input() -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::invalid_input_issues(
        "outbound delivery target input failed validation",
        vec![
            DispatchInputIssue::new("target_id", DispatchInputIssueCode::InvalidValue).expected(
                "an exact outbound delivery target id returned by builtin__outbound_delivery_targets_list",
            ),
        ],
    )
}
