//! Process services factory.
//!
//! `ironclaw_processes` is merged. The composition root wires the
//! in-memory `ProcessStore`, `ProcessResultStore`, and shared
//! `ProcessCancellationRegistry` into a [`RebornProcessServices`]
//! bundle so the capability host (when it lands) and the process host
//! read from the same handles — the contract from issue #3026
//! acceptance test #6 ("CapabilityHost and ProcessHost share the same
//! process store / result store / cancellation registry").
//!
//! Filesystem-backed `ProcessStore` / `ProcessResultStore` exist in
//! `ironclaw_processes` and can be wired through [`RebornProductionServices::filesystem_root`]
//! once typed settings select the backend. Until then, every profile
//! gets the in-memory pair and `Production` returns
//! [`crate::RebornBuildError::SubstrateNotImplemented`] with service
//! `durable_process_store` so a production build cannot accidentally
//! run on volatile state.
//!
//! [`RebornProcessServices`]: crate::RebornProcessServices
//! [`RebornProductionServices::filesystem_root`]: crate::RebornProductionServices

use std::sync::Arc;

use ironclaw_processes::{
    InMemoryProcessResultStore, InMemoryProcessStore, ProcessCancellationRegistry,
};

use crate::{RebornBuildError, RebornBuildInput, RebornProcessServices, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    let bundle = RebornProcessServices {
        cancellation: Arc::new(ProcessCancellationRegistry::new()),
        store: Arc::new(InMemoryProcessStore::new()),
        result_store: Arc::new(InMemoryProcessResultStore::new()),
    };
    services.process_services = Some(Arc::new(bundle));

    if input.profile == crate::RebornProfile::Production {
        return Err(RebornBuildError::SubstrateNotImplemented {
            service: "durable_process_store",
        });
    }

    Ok(())
}
