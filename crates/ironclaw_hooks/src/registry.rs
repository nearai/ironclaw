//! Hook registry — the per-run table of active hook bindings.
//!
//! Sourced from the active `RunProfile` (not from a global table) so that
//! hook composition is deterministic per run and replay refuses on version
//! drift. The skeleton in this PR exposes the binding shape and a simple
//! resolver; the actual `RunProfile.hooks` field and the manifest→binding
//! installer pipeline land in follow-up slices that touch
//! `ironclaw_turns::run_profile` and the extension installer.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::HookError;
use crate::identity::{HookId, HookVersion};
use crate::ordering::HookPhase;
use crate::trust::HookTrustClass;

/// A single hook registration for an active run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookBinding {
    pub hook_id: HookId,
    pub hook_version: HookVersion,
    pub trust_class: HookTrustClass,
    pub phase: HookPhase,
    /// Coarse description of where the hook fires. The actual hook
    /// implementation (the trait object) is stored separately so this type
    /// remains serializable for checkpoint payloads.
    pub point: HookPointSpec,
    /// `true` if the dispatcher poisoned this slot during the current run.
    /// Persisted so resume cannot re-enable a hook that already crashed.
    pub poisoned: bool,
}

/// Identifies which dispatcher point a binding registers against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPointSpec {
    BeforeCapability,
    BeforePrompt,
    AfterModel,
    AfterCapability,
    AfterCheckpoint,
}

/// Bindings grouped by dispatcher point for cheap lookup during a tick.
#[derive(Debug, Default)]
pub struct HookRegistry {
    by_point: HashMap<HookPointSpec, Vec<HookBinding>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from an iterator of bindings. Returns
    /// [`HookError::RegistryConstruction`] if any binding fails the
    /// phase-vs-trust gate (e.g., an Installed hook attempts to register at
    /// `Validation`).
    pub fn from_bindings<I>(bindings: I) -> Result<Self, HookError>
    where
        I: IntoIterator<Item = HookBinding>,
    {
        let mut registry = Self::new();
        for binding in bindings {
            registry.insert(binding)?;
        }
        Ok(registry)
    }

    pub fn insert(&mut self, binding: HookBinding) -> Result<(), HookError> {
        if !binding.phase.permits_trust(binding.trust_class) {
            return Err(HookError::RegistryConstruction(format!(
                "{:?}-tier hook cannot register at phase {:?}",
                binding.trust_class, binding.phase
            )));
        }
        self.by_point
            .entry(binding.point)
            .or_default()
            .push(binding);
        Ok(())
    }

    /// Active (non-poisoned) bindings at a point.
    pub fn active_at(&self, point: HookPointSpec) -> impl Iterator<Item = &HookBinding> {
        self.by_point
            .get(&point)
            .into_iter()
            .flat_map(|v| v.iter())
            .filter(|b| !b.poisoned)
    }

    /// Mark a hook's slot poisoned for the rest of the run.
    pub fn poison(&mut self, hook_id: HookId) {
        for bindings in self.by_point.values_mut() {
            for binding in bindings.iter_mut() {
                if binding.hook_id == hook_id {
                    binding.poisoned = true;
                }
            }
        }
    }

    pub fn is_poisoned(&self, hook_id: HookId) -> bool {
        self.by_point
            .values()
            .flat_map(|bindings| bindings.iter())
            .any(|b| b.hook_id == hook_id && b.poisoned)
    }

    /// Total number of bindings, poisoned or not.
    pub fn len(&self) -> usize {
        self.by_point.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.by_point.values().all(Vec::is_empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ExtensionId, HookLocalId};

    fn installed_binding(local: &str, phase: HookPhase, point: HookPointSpec) -> HookBinding {
        let hook_id = HookId::derive(
            &ExtensionId("ext".to_string()),
            "1.0",
            &HookLocalId(local.to_string()),
            HookVersion::ONE,
        );
        HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase,
            point,
            poisoned: false,
        }
    }

    #[test]
    fn rejects_installed_at_validation_phase() {
        let mut registry = HookRegistry::new();
        let result = registry.insert(installed_binding(
            "alpha",
            HookPhase::Validation,
            HookPointSpec::BeforeCapability,
        ));
        match result {
            Err(HookError::RegistryConstruction(msg)) => {
                assert!(msg.contains("Validation"));
                assert!(msg.contains("Installed"));
            }
            other => panic!("expected registry construction error, got {other:?}"),
        }
    }

    #[test]
    fn accepts_installed_at_policy_phase() {
        let mut registry = HookRegistry::new();
        registry
            .insert(installed_binding(
                "alpha",
                HookPhase::Policy,
                HookPointSpec::BeforeCapability,
            ))
            .expect("policy phase is open to Installed");
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn poisoned_hooks_are_filtered_from_active() {
        let mut registry = HookRegistry::new();
        let binding =
            installed_binding("alpha", HookPhase::Policy, HookPointSpec::BeforeCapability);
        let id = binding.hook_id;
        registry.insert(binding).expect("ok");
        assert_eq!(
            registry.active_at(HookPointSpec::BeforeCapability).count(),
            1
        );

        registry.poison(id);
        assert_eq!(
            registry.active_at(HookPointSpec::BeforeCapability).count(),
            0
        );
        assert!(registry.is_poisoned(id));
    }
}
