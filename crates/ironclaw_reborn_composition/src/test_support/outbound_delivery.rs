//! `outbound_delivery_*` synthetic-capability test support (C-SYNTH outbound
//! seam).

/// Capability id of the local-dev synthetic `outbound_delivery_targets_list`
/// capability. Single owner is the production constant in
/// `outbound_delivery_capability_surface`; the harness references this so its
/// `outbound_target_tools()` constructor and assertions never hardcode the
/// string.
#[cfg(feature = "test-support")]
pub const OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID: &str =
    crate::outbound::OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID;
/// Capability id of the local-dev synthetic `outbound_delivery_target_set`
/// capability. See [`OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID`].
#[cfg(feature = "test-support")]
pub const OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID: &str =
    crate::outbound::OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID;
