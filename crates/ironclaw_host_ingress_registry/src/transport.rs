use ironclaw_host_api::{
    HostApiError, IngressAckMode, IngressDrainMode, IngressPolicy, IngressRouteDescriptor,
    IngressRouteId, IngressRoutePattern, NetworkMethod,
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

impl HostIngressTransport {
    pub(crate) fn into_descriptor(
        self,
        policy: IngressPolicy,
    ) -> Result<(IngressRouteDescriptor, IngressAckMode, IngressDrainMode), HostApiError> {
        match self {
            Self::Webhook {
                route_id,
                method,
                path,
                ack,
                drain,
            } => {
                let descriptor =
                    IngressRouteDescriptor::new(route_id.as_str(), method, path.as_str(), policy)?;
                Ok((descriptor, ack, drain))
            }
        }
    }
}
