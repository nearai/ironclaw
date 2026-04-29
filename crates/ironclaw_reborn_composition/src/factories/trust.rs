//! Trust-class policy engine factory.
//!
//! Tracked by #3012 / #3043. Trust class assignment is host-controlled — a
//! user-installed manifest cannot self-promote to `FirstParty`/`System`. The
//! authorization, approval, dispatcher and extension factories all need a
//! validated trust input before they can produce a fail-closed graph, so
//! `Production` fails here until #3043 merges and we replace this gate with a
//! real builder over the policy engine.

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    _services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    super::gate_substrate(input, "trust_class_policy")
}
