//! Reborn network policy enforcer factory.
//!
//! `ironclaw_network` is merged. The composition root wires
//! [`ironclaw_network::StaticNetworkPolicyEnforcer`] over a default
//! [`ironclaw_host_api::NetworkPolicy`] (deny-all: no `allowed_targets`,
//! private IPs not denied, no egress byte limit). With an empty
//! `allowed_targets` list the network crate fails closed for every
//! request — the safe starting point until typed settings overlay a
//! configured policy.
//!
//! A live `NetworkPolicyStore` (per-scope policy persistence with
//! PG/libSQL backends) lands with the second composition phase
//! (`reborn.network.policy_backend` from issue #3026's config-model
//! section). Until then any profile that requires the full graph
//! (Production *or* MigrationDryRun) returns
//! [`crate::RebornBuildError::SubstrateNotImplemented`] with service
//! `durable_network_policy_backend` because a deny-all default is not
//! a cutover-ready policy — the operator must configure allowed
//! targets before live traffic can flow, and a dry run must catch the
//! misconfiguration rather than silently boot.

use std::sync::Arc;

use ironclaw_host_api::NetworkPolicy;
use ironclaw_network::StaticNetworkPolicyEnforcer;

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    let enforcer = Arc::new(StaticNetworkPolicyEnforcer::new(NetworkPolicy::default()));
    services.network_enforcer = Some(enforcer);

    if input.profile.requires_full_graph() {
        return Err(RebornBuildError::SubstrateNotImplemented {
            service: "durable_network_policy_backend",
        });
    }

    Ok(())
}
