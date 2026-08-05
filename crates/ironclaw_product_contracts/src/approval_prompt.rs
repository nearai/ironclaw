//! The store-free half of approval-prompt rendering.
//!
//! Three pure pieces every approval-prompt reader needs and none of which
//! requires the approval *store*: the gate-ref parse, the lookup scope, and the
//! projection of an [`ApprovalRequest`] onto the redacted
//! [`ApprovalPromptContextView`] the wire carries.
//!
//! **Why here.** Every input and output is `ironclaw_host_api` vocabulary plus
//! this crate's own view family, and there are two consumers that must not
//! import one another: `ironclaw_assistant`'s projection layer and
//! `ironclaw_extension_host`'s `ApprovalPromptContextSource` implementation
//! (`crate::prompt_source`). Before WS2.5 the extension host reached *up* into
//! `ironclaw_assistant::projection::approval_prompt_context_view` for exactly
//! this, and product carried the projection twice — once in `approval_prompt.rs`
//! and again in `projection/turn_events.rs`. One definition, here.
//!
//! Never here: the store read itself. `ApprovalRequestStorePort` lives in
//! `ironclaw_approvals` (kernel) and a contracts crate may not name it — which
//! is the boundary that decides the split: whoever holds the store does the
//! `get` and calls these three.

use ironclaw_host_api::action::{Action, NetworkMethod, NetworkScheme};
use ironclaw_host_api::approval::ApprovalRequest;
use ironclaw_host_api::ids::{ApprovalRequestId, InvocationId, UserId};
use ironclaw_host_api::resource::ResourceScope;
use ironclaw_host_api::turn::{TurnGateRef, TurnScope};

use crate::outbound::{
    ApprovalPromptActionView, ApprovalPromptContextView, ApprovalPromptDestinationView,
    ApprovalPromptDetailView, ApprovalPromptScopeView,
};

/// The gate-ref prefix every approval gate carries.
pub const APPROVAL_GATE_PREFIX: &str = "gate:approval-";

/// Whether a raw gate-ref string names an approval gate.
pub fn is_approval_gate_ref(gate_ref: &str) -> bool {
    gate_ref.starts_with(APPROVAL_GATE_PREFIX)
}

/// The approval request behind an approval gate ref, or `None` when the ref is
/// not an approval ref or its id does not parse.
///
/// Deliberately `Option`, not `Result`: this crate has no approval-rejection
/// vocabulary, and both prompt readers already discard the reason. Callers that
/// need a typed rejection wrap it — `ironclaw_assistant`'s
/// `approval_request_id_from_gate_ref` does exactly that.
pub fn approval_request_id_from_gate_ref(gate_ref: &TurnGateRef) -> Option<ApprovalRequestId> {
    let value = gate_ref.as_str().strip_prefix(APPROVAL_GATE_PREFIX)?;
    ApprovalRequestId::parse(value).ok()
}

/// The resource scope an approval-prompt lookup reads under.
///
/// An explicit turn owner (a shared/team subject) wins over the acting user,
/// matching `ApprovalInteractionScope::from_turn`; the equivalence of the two
/// is pinned in `ironclaw_assistant`
/// (`approval_prompt_lookup_scope_matches_the_interaction_scope_projection`).
pub fn approval_prompt_lookup_scope(
    turn_scope: &TurnScope,
    owner_user_id: &UserId,
) -> ResourceScope {
    ResourceScope {
        tenant_id: turn_scope.tenant_id.clone(),
        user_id: turn_scope
            .explicit_owner_user_id()
            .cloned()
            .unwrap_or_else(|| owner_user_id.clone()),
        agent_id: turn_scope.agent_id.clone(),
        project_id: turn_scope.project_id.clone(),
        mission_id: None,
        thread_id: Some(turn_scope.thread_id.clone()),
        invocation_id: InvocationId::new(),
    }
}

pub fn approval_prompt_context_for_request(
    request: &ApprovalRequest,
) -> Option<ApprovalPromptContextView> {
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
