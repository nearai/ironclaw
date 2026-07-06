//! Network policy and HTTP egress boundary for IronClaw Reborn.
//!
//! This crate evaluates host API [`NetworkPolicy`] values against scoped network
//! requests, resolves DNS, rejects private resolved targets when configured,
//! and owns outbound HTTP transport for host-mediated runtime requests. It does
//! not inject secrets, reserve resources, emit audit/events, or run product
//! workflow.
#![warn(unreachable_pub)]

mod egress;
mod error;
mod policy;
mod resolver;
mod transport;
mod types;
mod url_target;

pub use egress::{NetworkHttpEgress, NetworkHttpTransport, PolicyNetworkHttpEgress};
pub use error::NetworkHttpError;
pub use policy::{StaticNetworkPolicyEnforcer, target_matches_pattern};
pub use resolver::NetworkResolver;
pub use transport::ReqwestNetworkTransport;
pub use types::{
    DEFAULT_RESPONSE_BODY_LIMIT, NetworkHttpRequest, NetworkHttpResponse, NetworkRequest,
    NetworkTransportRequest, NetworkUsage,
};
pub use url_target::{
    NetworkTargetUrlError, is_rfc3986_unreserved_segment, network_target_for_url,
    percent_decode_url_component_lossy,
};
