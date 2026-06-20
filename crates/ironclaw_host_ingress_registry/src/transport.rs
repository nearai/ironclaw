use ironclaw_host_api::{
    IngressAckMode, IngressDrainMode, IngressRouteId, IngressRoutePattern, NetworkMethod,
};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum HostIngressTransport {
    Webhook {
        route_id: IngressRouteId,
        method: NetworkMethod,
        path: IngressRoutePattern,
        ack: IngressAckMode,
        drain: IngressDrainMode,
    },
}
