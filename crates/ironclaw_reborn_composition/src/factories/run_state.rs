//! Run-state and approval-request store factory.
//!
//! `ironclaw_run_state` is merged. In-memory stores are wired for every
//! non-disabled profile. The persistent Postgres / libSQL backends will
//! replace these here once those factories land in a follow-up PR; until
//! then `Production` flips into the `durable_run_state_backend` gate at the
//! end of this build step.

use std::sync::Arc;

use ironclaw_run_state::{InMemoryApprovalRequestStore, InMemoryRunStateStore};

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    services.run_state_store = Some(Arc::new(InMemoryRunStateStore::new()));
    services.approval_request_store = Some(Arc::new(InMemoryApprovalRequestStore::new()));

    if input.profile == crate::RebornProfile::Production {
        return Err(RebornBuildError::SubstrateNotImplemented {
            service: "durable_run_state_backend",
        });
    }

    Ok(())
}
