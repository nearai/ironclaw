use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{AllowedEffectPath, AuditTraceClass, IngressRouteDescriptor};

use super::{body_limit_kib, descriptor, mutation_policy, mutation_rate_limit};

pub const WEBUI_V2_ROUTE_CANCEL_RUN: &str = "webui.v2.cancel_run";
pub const WEBUI_V2_ROUTE_RESOLVE_GATE: &str = "webui.v2.resolve_gate";
pub const WEBUI_V2_ROUTE_RETRY_RUN: &str = "webui.v2.retry_run";

pub const WEBUI_V2_PATTERN_CANCEL_RUN: &str =
    "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel";
pub const WEBUI_V2_PATTERN_RESOLVE_GATE: &str =
    "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/gates/{gate_ref}/resolve";
pub const WEBUI_V2_PATTERN_RETRY_RUN: &str =
    "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/retry";

pub(super) fn cancel_run_descriptor() -> IngressRouteDescriptor {
    run_action_descriptor(WEBUI_V2_ROUTE_CANCEL_RUN, WEBUI_V2_PATTERN_CANCEL_RUN)
}

pub(super) fn resolve_gate_descriptor() -> IngressRouteDescriptor {
    run_action_descriptor(WEBUI_V2_ROUTE_RESOLVE_GATE, WEBUI_V2_PATTERN_RESOLVE_GATE)
}

pub(super) fn retry_run_descriptor() -> IngressRouteDescriptor {
    run_action_descriptor(WEBUI_V2_ROUTE_RETRY_RUN, WEBUI_V2_PATTERN_RETRY_RUN)
}

fn run_action_descriptor(route_id: &str, pattern: &str) -> IngressRouteDescriptor {
    descriptor(
        route_id,
        NetworkMethod::Post,
        pattern,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::TurnCoordinator,
        ),
    )
}
