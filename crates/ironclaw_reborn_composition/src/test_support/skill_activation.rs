//! `skill_activate` synthetic-capability test support (E-SKILL seam).

/// Capability id of the local-dev synthetic `skill_activate` capability
/// (E-SKILL seam). Single owner is the production constant in
/// `runtime::local_dev::skill_activation`; mirrors `PROJECT_CREATE_CAPABILITY_ID`.
#[cfg(feature = "test-support")]
pub const SKILL_ACTIVATE_CAPABILITY_ID: &str = crate::runtime::SKILL_ACTIVATE_CAPABILITY_ID;

/// Opaque handle (E-SKILL seam) carrying the built local-dev skill context
/// source. Hides the crate-private `ComposedSelectableSkillContextSource` from
/// the integration-test crate, which cannot name it. Exposes the
/// `HostSkillContextSource` for runtime wiring; the activation source travels
/// inside for the harness's `RefreshingCapabilityPortTestParts`. Tests
/// only.
#[cfg(feature = "test-support")]
pub struct SkillActivationTestSource {
    source: std::sync::Arc<dyn ironclaw_loop_host::HostSkillContextSource>,
    activation_source: std::sync::Arc<crate::runtime::ComposedSelectableSkillContextSource>,
}

#[cfg(feature = "test-support")]
impl SkillActivationTestSource {
    /// The `HostSkillContextSource` to wire as the runtime's
    /// `skill_context_source` (`into_group`, E-SKILL) so activated-skill
    /// instructions inject into the model request.
    pub fn context_source(&self) -> std::sync::Arc<dyn ironclaw_loop_host::HostSkillContextSource> {
        self.source.clone()
    }

    /// Crate-internal accessor for the wrapped activation source. Kept
    /// `pub(crate)` (never `pub`) so the crate-private
    /// `ComposedSelectableSkillContextSource` type never appears in this
    /// crate's public API; only `runtime::local_dev`'s test-support
    /// constructor (which already names the type) may call this.
    pub(crate) fn activation_source(
        &self,
    ) -> std::sync::Arc<crate::runtime::ComposedSelectableSkillContextSource> {
        self.activation_source.clone()
    }
}

/// Build the local-dev skill context source (`HostSkillContextSource` for
/// prompt injection plus the activation source backing `skill_activate`) over
/// the runtime's skill filesystem, mirroring production `build_reborn_runtime`
/// wiring (runtime.rs ~line 2875). Returns `None` when no local runtime is
/// composed. Mirrors `build_user_profile_source_for_test` (E-SKILL seam).
/// Tests only.
#[cfg(feature = "test-support")]
pub fn build_skill_context_source_for_test(
    runtime: &crate::RebornRuntime,
    _tenant_id: &ironclaw_host_api::TenantId,
    _regex_skill_activation_enabled: bool,
) -> Option<SkillActivationTestSource> {
    Some(SkillActivationTestSource {
        source: runtime.skill_context_source.clone()?,
        activation_source: runtime.skill_activation_source.clone()?,
    })
}
