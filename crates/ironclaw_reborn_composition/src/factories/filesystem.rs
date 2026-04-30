//! Filesystem root factory.
//!
//! `ironclaw_filesystem` is merged. The composition root currently wires a
//! [`CompositeRootFilesystem`] with no mounts under every non-disabled
//! profile so downstream substrate (`run_state`, `extensions`) has a root to
//! resolve against. Local/Postgres/libSQL filesystem backends are added
//! through `mount_dyn` once the persistent store factories land.

use std::sync::Arc;

use ironclaw_filesystem::CompositeRootFilesystem;

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    _input: &RebornBuildInput,
    services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    let root = Arc::new(CompositeRootFilesystem::new());
    services.filesystem_root = Some(root);
    Ok(())
}
