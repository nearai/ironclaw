//! Resource governor factory.
//!
//! `ironclaw_resources` is already merged. Production currently has only the
//! `InMemoryResourceGovernor` reference implementation — a durable per-account
//! ledger backend is tracked separately and will replace the in-memory variant
//! once the persistent store factories land. Until then, every profile uses
//! the in-memory governor; the gate against in-memory backends in
//! `Production` will move here when the durable variant exists.

use std::sync::Arc;

use ironclaw_resources::InMemoryResourceGovernor;

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    _input: &RebornBuildInput,
    services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    services.resource_governor = Some(governor);
    Ok(())
}
