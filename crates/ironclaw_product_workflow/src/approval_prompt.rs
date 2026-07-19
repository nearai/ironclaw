//! Shared approval prompt lookup and redacted context projection.

use ironclaw_host_api::{
    Action, ApprovalRequest, InvocationId, NetworkMethod, NetworkScheme, UserId,
};
use ironclaw_product_adapters::{
    ApprovalPromptActionView, ApprovalPromptContextView, ApprovalPromptDestinationView,
    ApprovalPromptDetailView, ApprovalPromptScopeView,
};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_run_state::RunStateError;
use ironclaw_turns::{GateRef, TurnActor, TurnScope};
use thiserror::Error;

use crate::{ApprovalInteractionScope, approval_request_id_from_gate_ref};

#[derive(Debug, Default)]
pub struct ApprovalPromptLookup {
    pub context: Option<ApprovalPromptContextView>,
    pub invocation_id: Option<InvocationId>,
}

#[derive(Debug, Error)]
#[error("approval prompt context is temporarily unavailable")]
pub struct ApprovalPromptLookupError {
    #[source]
    source: RunStateError,
}

pub async fn approval_prompt_lookup(
    approval_requests: Option<&dyn ApprovalRequestStore>,
    gate_ref: &GateRef,
    owner_user_id: &UserId,
    turn_scope: &TurnScope,
) -> Result<ApprovalPromptLookup, ApprovalPromptLookupError> {
    let (store, request_id) =
        match approval_requests.zip(approval_request_id_from_gate_ref(gate_ref).ok()) {
            Some(value) => value,
            None => return Ok(ApprovalPromptLookup::default()),
        };
    let scope =
        ApprovalInteractionScope::from_turn(turn_scope, &TurnActor::new(owner_user_id.clone()))
            .to_resource_scope();
    match store.get(&scope, request_id).await {
        Ok(Some(record)) => Ok(ApprovalPromptLookup {
            context: approval_context_for_request(&record.request),
            invocation_id: Some(record.scope.invocation_id),
        }),
        Ok(None) => Ok(ApprovalPromptLookup::default()),
        Err(source) => Err(ApprovalPromptLookupError { source }),
    }
}

pub async fn approval_prompt_context_view(
    approval_requests: Option<&dyn ApprovalRequestStore>,
    gate_ref: &GateRef,
    owner_user_id: &UserId,
    turn_scope: &TurnScope,
) -> Result<Option<ApprovalPromptContextView>, ApprovalPromptLookupError> {
    approval_prompt_lookup(approval_requests, gate_ref, owner_user_id, turn_scope)
        .await
        .map(|lookup| lookup.context)
}

fn approval_context_for_request(request: &ApprovalRequest) -> Option<ApprovalPromptContextView> {
    let (tool_name, action, destination, details) =
        approval_action_context(request.action.as_ref())?;
    ApprovalPromptContextView::new(
        tool_name,
        action,
        ApprovalPromptScopeView::new(
            approval_scope_label(request),
            request.reusable_scope.is_some(),
        )
        .ok()?,
        non_empty_string(&request.reason),
        destination,
        details,
    )
    .ok()
}

fn approval_action_context(
    action: &Action,
) -> Option<(
    String,
    ApprovalPromptActionView,
    Option<ApprovalPromptDestinationView>,
    Vec<ApprovalPromptDetailView>,
)> {
    match action {
        Action::Dispatch {
            capability,
            estimated_resources,
        } => {
            let mut details = vec![detail("Capability", capability.as_str())?];
            if let Some(bytes) = estimated_resources.network_egress_bytes {
                details.push(detail("Estimated network egress", format_bytes(bytes))?);
            }
            Some((
                capability.as_str().to_string(),
                ApprovalPromptActionView::new("Run tool", None).ok()?,
                None,
                details,
            ))
        }
        Action::SpawnCapability {
            capability,
            estimated_resources,
        } => {
            let mut details = vec![detail("Capability", capability.as_str())?];
            if let Some(process_count) = estimated_resources.process_count {
                details.push(detail("Processes", process_count.to_string())?);
            }
            Some((
                capability.as_str().to_string(),
                ApprovalPromptActionView::new("Start tool", None).ok()?,
                None,
                details,
            ))
        }
        Action::Network {
            target,
            method,
            estimated_bytes,
        } => {
            let destination =
                network_destination(method, target.scheme, &target.host, target.port)?;
            let mut details = vec![detail("Method", method_label(method))?];
            if let Some(bytes) = estimated_bytes {
                details.push(detail("Estimated transfer", format_bytes(*bytes))?);
            }
            Some((
                "builtin.http".to_string(),
                ApprovalPromptActionView::new("Network request", Some(*method)).ok()?,
                Some(destination),
                details,
            ))
        }
        _ => None,
    }
}

fn approval_scope_label(request: &ApprovalRequest) -> &'static str {
    if request.reusable_scope.is_some() {
        "Reusable grant"
    } else {
        "This request only"
    }
}

fn network_destination(
    method: &NetworkMethod,
    scheme: NetworkScheme,
    host: &str,
    port: Option<u16>,
) -> Option<ApprovalPromptDestinationView> {
    let scheme = match scheme {
        NetworkScheme::Http => "http",
        NetworkScheme::Https => "https",
    };
    let authority = match port {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    let url = format!("{scheme}://{authority}");
    ApprovalPromptDestinationView::new(
        format!("{} {url}", method_label(method)),
        Some(url),
        Some(host.to_string()),
    )
    .ok()
}

fn detail(label: impl Into<String>, value: impl Into<String>) -> Option<ApprovalPromptDetailView> {
    ApprovalPromptDetailView::new(label, value).ok()
}

fn method_label(method: &NetworkMethod) -> String {
    method.to_string().to_ascii_uppercase()
}

fn format_bytes(bytes: u64) -> String {
    format!("{bytes} bytes")
}

fn non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
