//! `project_create` synthetic-capability test support (E-PROJ seam).

/// Capability id of the local-dev synthetic `project_create` capability
/// (E-PROJ seam). Single owner is the production constant in
/// `runtime::local_dev::project_create`; the harness references this so its
/// `project_tools()` constructor and assertions never hardcode the string.
#[cfg(feature = "test-support")]
pub const PROJECT_CREATE_CAPABILITY_ID: &str = crate::runtime::PROJECT_CREATE_CAPABILITY_ID;
