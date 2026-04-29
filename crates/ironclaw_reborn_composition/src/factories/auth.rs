//! Authorization and capability lease store factory.
//!
//! `ironclaw_authorization` is merged. The composition root wires the
//! grant-backed `CapabilityDispatchAuthorizer` and an in-memory lease store
//! that the approval resolver shares.
//!
//! `LeaseBackedAuthorizer` is intentionally **not** wired here: it borrows
//! its lease store with a lifetime parameter and so cannot be stored as
//! `Arc<dyn CapabilityDispatchAuthorizer>`. The approval-resolution slice
//! that needs it builds the borrowed authorizer at call time over the
//! shared store handle on `RebornProductionServices`.

use std::sync::Arc;

use ironclaw_authorization::{GrantAuthorizer, InMemoryCapabilityLeaseStore};

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    _input: &RebornBuildInput,
    services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    services.authorization = Some(Arc::new(GrantAuthorizer::new()));
    services.capability_lease_store = Some(Arc::new(InMemoryCapabilityLeaseStore::new()));
    Ok(())
}
