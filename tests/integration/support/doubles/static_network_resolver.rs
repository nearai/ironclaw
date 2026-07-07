/// Fake DNS resolver for the real-egress-pipeline seam
/// (`RecordingNetworkHttpTransport`): resolves any host to a fixed public IP
/// so the real `PolicyNetworkHttpEgress`'s DNS/private-IP check runs for real
/// without making a real DNS lookup.
use std::net::IpAddr;

use ironclaw_network::{NetworkHttpError, NetworkResolver};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StaticNetworkResolver;

impl NetworkResolver for StaticNetworkResolver {
    fn resolve_ips(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>, NetworkHttpError> {
        Ok(vec!["93.184.216.34".parse().expect("valid IP literal")])
    }
}
