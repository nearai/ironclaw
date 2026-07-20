//! `HostUserProfileSource` test support (E-PROFILE seam).

/// Build the `HostUserProfileSource` the Reborn integration harness wires into
/// the group's single planned runtime in `into_group`
/// (`tests/support/reborn/group.rs`, E-PROFILE seam).
///
/// Reuses the production `MemoryBackedUserProfileSourceAdapter` (the single
/// orphan-rule wrapper around `MemoryBackedUserProfileSource`) so the test path
/// never drifts from production wiring (runtime.rs ~line 3167). When `filesystem`
/// is `Some`, profile reads flow through the native memory service over that
/// raw local-dev memory filesystem; `None` (non-HostRuntime backends) falls back
/// to `EmptyUserProfileSource`.
#[cfg(feature = "test-support")]
pub fn build_user_profile_source_for_test(
    filesystem: Option<std::sync::Arc<dyn ironclaw_filesystem::RootFilesystem>>,
) -> std::sync::Arc<dyn ironclaw_loop_host::HostUserProfileSource> {
    match filesystem {
        Some(fs) => std::sync::Arc::new(crate::runtime::MemoryBackedUserProfileSourceAdapter(
            ironclaw_host_runtime::MemoryBackedUserProfileSource::from_filesystem(fs),
        )),
        None => std::sync::Arc::new(ironclaw_loop_host::EmptyUserProfileSource),
    }
}
