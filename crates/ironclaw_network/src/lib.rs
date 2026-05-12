//! Network policy and HTTP egress boundary for IronClaw Reborn.
//!
//! This crate evaluates host API [`NetworkPolicy`] values against scoped network
//! requests, resolves DNS, rejects private resolved targets when configured,
//! and owns outbound HTTP transport for host-mediated runtime requests. It does
//! not inject secrets, reserve resources, emit audit/events, or run product
//! workflow.

mod egress;
mod error;
mod policy;
mod resolver;
mod transport;
mod types;
mod url_target;

pub use egress::{NetworkHttpEgress, NetworkHttpTransport, PolicyNetworkHttpEgress};
pub use error::{NetworkHttpError, NetworkHttpErrorKind};
pub use policy::{
    NetworkPolicyEnforcer, NetworkPolicyError, StaticNetworkPolicyEnforcer, host_matches_pattern,
    is_private_or_loopback_ip, network_policy_allows, target_matches_pattern,
};
pub use resolver::{NetworkResolver, SystemNetworkResolver};
pub use transport::ReqwestNetworkTransport;
pub use types::{
    DEFAULT_RESPONSE_BODY_LIMIT, NetworkHttpRequest, NetworkHttpResponse, NetworkPermit,
    NetworkRequest, NetworkTransportRequest, NetworkUsage,
};
pub use url_target::{default_port, network_target_for_url, scheme_label};
